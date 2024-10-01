use super::{handshake, Network, ValidatorAddrs};
use crate::{noise, preface, rpc};
use anyhow::Context as _;
use async_trait::async_trait;
use rand::seq::SliceRandom;
use std::sync::atomic::Ordering;
use zksync_concurrency::{ctx, net, scope, sync};
use zksync_consensus_roles::node;
use zksync_consensus_storage::{BlockStore, BlockStoreState};
use zksync_protobuf::kB;

/// Receiver of push messages from the peers.
struct PushServer<'a> {
    blocks: sync::watch::Sender<BlockStoreState>,
    net: &'a Network,
}

impl<'a> PushServer<'a> {
    fn new(net: &'a Network) -> Self {
        Self {
            blocks: sync::watch::channel(BlockStoreState {
                first: net.genesis().first_block,
                last: None,
            })
            .0,
            net,
        }
    }
}

#[async_trait]
impl rpc::Handler<rpc::push_validator_addrs::Rpc> for &PushServer<'_> {
    fn max_req_size(&self) -> usize {
        100 * kB
    }
    async fn handle(
        &self,
        _ctx: &ctx::Ctx,
        req: rpc::push_validator_addrs::Req,
    ) -> anyhow::Result<()> {
        self.net
            .push_validator_addrs_calls
            .fetch_add(1, Ordering::SeqCst);
        self.net
            .validator_addrs
            .update(&self.net.genesis().validators, &req.0)
            .await?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl rpc::Handler<rpc::push_batch_votes::Rpc> for &PushServer<'_> {
    fn max_req_size(&self) -> usize {
        100 * kB
    }

    async fn handle(
        &self,
        _ctx: &ctx::Ctx,
        req: rpc::push_batch_votes::Req,
    ) -> anyhow::Result<rpc::push_batch_votes::Resp> {
        if let Err(err) = self
            .net
            .attestation
            .insert_votes(req.votes.into_iter())
            .await
            .context("insert_votes()")
        {
            // Attestation feature is still evolving, so for forward
            // compatibility we just ignore any invalid data.
            // Once stabilized we will drop the connection instead of
            // logging the error.
            tracing::warn!("{err:#}");
        }
        Ok(rpc::push_batch_votes::Resp {
            votes: match req.want_votes_for.as_ref() {
                Some(batch) => self.net.attestation.votes(batch),
                None => vec![],
            },
        })
    }
}

#[async_trait]
impl rpc::Handler<rpc::push_block_store_state::Rpc> for &PushServer<'_> {
    fn max_req_size(&self) -> usize {
        10 * kB
    }
    async fn handle(
        &self,
        _ctx: &ctx::Ctx,
        mut req: rpc::push_block_store_state::Req,
    ) -> anyhow::Result<()> {
        if !self.net.cfg.enable_pre_genesis_support {
            req.clear_pre_genesis_info();
        }
        let state = req.state();
        state.verify(self.net.genesis())?;
        self.blocks.send_replace(state);
        Ok(())
    }
}

#[async_trait]
impl rpc::Handler<rpc::get_block::Rpc> for &BlockStore {
    fn max_req_size(&self) -> usize {
        kB
    }
    async fn handle(
        &self,
        ctx: &ctx::Ctx,
        req: rpc::get_block::Req,
    ) -> anyhow::Result<rpc::get_block::Resp> {
        let mut resp = rpc::get_block::Resp(self.block(ctx, req.0).await?;
        if !self.cfg.enable_pre_genesis_support { 
            resp.clear_pre_genesis_info();
        }
        Ok(resp)
    }
}

impl Network {
    /// Manages lifecycle of a single connection.
    async fn run_stream(&self, ctx: &ctx::Ctx, stream: noise::Stream) -> anyhow::Result<()> {
        let push_server = PushServer::new(self);
        let push_validator_addrs_client = rpc::Client::<rpc::push_validator_addrs::Rpc>::new(
            ctx,
            self.cfg.rpc.push_validator_addrs_rate,
        );
        let push_block_store_state_client = rpc::Client::<rpc::push_block_store_state::Rpc>::new(
            ctx,
            self.cfg.rpc.push_block_store_state_rate,
        );
        let get_block_client =
            rpc::Client::<rpc::get_block::Rpc>::new(ctx, self.cfg.rpc.get_block_rate);
        let push_batch_votes_client =
            rpc::Client::<rpc::push_batch_votes::Rpc>::new(ctx, self.cfg.rpc.push_batch_votes_rate);

        scope::run!(ctx, |ctx, s| async {
            let mut service = rpc::Service::new()
                .add_client(&push_validator_addrs_client)
                .add_server::<rpc::push_validator_addrs::Rpc>(
                    ctx,
                    &push_server,
                    self.cfg.rpc.push_validator_addrs_rate,
                )
                .add_client(&push_block_store_state_client)
                .add_server::<rpc::push_block_store_state::Rpc>(
                    ctx,
                    &push_server,
                    self.cfg.rpc.push_block_store_state_rate,
                )
                .add_client(&get_block_client)
                .add_server(ctx, &*self.block_store, self.cfg.rpc.get_block_rate)
                .add_server(ctx, rpc::ping::Server, rpc::ping::RATE)
                .add_client(&push_batch_votes_client)
                .add_server::<rpc::push_batch_votes::Rpc>(
                    ctx,
                    &push_server,
                    self.cfg.rpc.push_batch_votes_rate,
                );

            // Push L1 batch votes updates to peer.
            s.spawn::<()>(async {
                let push_batch_votes_client = push_batch_votes_client;
                // Subscribe to what we know about the state of the whole network.
                let mut recv = self.attestation.subscribe();
                loop {
                    let diff = recv.wait_for_diff(ctx).await?;
                    let req = rpc::push_batch_votes::Req {
                        // If the info has changed, we need to re-request all the votes
                        // from peer that we might have ignored earlier.
                        want_votes_for: diff.info.as_ref().map(|c| c.batch_to_attest.clone()),
                        votes: diff.votes,
                    };
                    // NOTE: The response should be non-empty only iff we requested a snapshot.
                    // Therefore, if we needed we could restrict the response size to ~1kB in
                    // such a case.
                    let resp = push_batch_votes_client.call(ctx, &req, 100 * kB).await?;
                    if !resp.votes.is_empty() {
                        anyhow::ensure!(
                            req.want_votes_for.is_some(),
                            "expected empty response, but votes were returned"
                        );
                        if let Err(err) = self
                            .attestation
                            .insert_votes(resp.votes.into_iter())
                            .await
                            .context("insert_votes")
                        {
                            // Attestation feature is still evolving, so for forward
                            // compatibility we just ignore any invalid data.
                            // Once stabilized we will drop the connection instead of
                            // logging the error.
                            tracing::warn!("{err:#}");
                        }
                    }
                }
            });

            if let Some(ping_timeout) = &self.cfg.ping_timeout {
                let ping_client = rpc::Client::<rpc::ping::Rpc>::new(ctx, rpc::ping::RATE);
                service = service.add_client(&ping_client);
                s.spawn(async {
                    let ping_client = ping_client;
                    ping_client.ping_loop(ctx, *ping_timeout).await
                });
            }

            // Push block store state updates to peer.
            s.spawn::<()>(async {
                let mut state = self.block_store.queued();
                loop {
                    let mut req = rpc::push_block_store_state::Req::new(state.clone(), self.genesis());
                    if !self.cfg.enable_pre_genesis_support {
                        req.clear_pre_genesis_info();
                    }
                    push_block_store_state_client.call(ctx, &req, kB).await?;
                    state = self.block_store.wait_for_queued_change(ctx, &state).await?;
                }
            });

            // Push validator addrs updates to peer.
            s.spawn::<()>(async {
                let mut old = ValidatorAddrs::default();
                let mut sub = self.validator_addrs.subscribe();
                sub.mark_changed();
                loop {
                    let new = sync::changed(ctx, &mut sub).await?.clone();
                    let diff = new.get_newer(&old);
                    if diff.is_empty() {
                        continue;
                    }
                    old = new;
                    let req = rpc::push_validator_addrs::Req(diff);
                    push_validator_addrs_client.call(ctx, &req, kB).await?;
                }
            });

            // Perform get_block calls to peer.
            s.spawn::<()>(async {
                // Gossiped state of what range of blocks is available on the remote peer.
                let state = &mut push_server.blocks.subscribe();
                loop {
                    let call = get_block_client.reserve(ctx).await?;
                    let (req, send_resp) = self.fetch_queue.accept_block(ctx, state).await?;
                    let req = rpc::get_block::Req(req);
                    s.spawn(async {
                        let req = req;
                        // Failing to fetch a block causes a disconnect:
                        // - peer predeclares which blocks are available and race condition
                        //   with block pruning should be very rare, so we can consider
                        //   an empty response to be offending
                        // - a stream for the call has been already reserved,
                        //   so the peer is expected to answer immediately. The timeout
                        //   should be high enough to accommodate network hiccups
                        // - a disconnect is not a ban, so the peer is free to try to
                        //   reconnect.
                        async {
                            let ctx_with_timeout =
                                self.cfg.rpc.get_block_timeout.map(|t| ctx.with_timeout(t));
                            let ctx = ctx_with_timeout.as_ref().unwrap_or(ctx);
                            let mut resp = call
                                .call(ctx, &req, self.cfg.max_block_size.saturating_add(kB))
                                .await?;
                            if self.cfg.enable_pre_genesis_support {
                                resp.0.clear_pre_genesis_info();
                            }
                            let block = resp.0.context("empty response")?;
                            anyhow::ensure!(block.number() == req.0, "received wrong block");
                            // Storing the block will fail in case block is invalid.
                            self.block_store
                                .queue_block(ctx, block)
                                .await
                                .context("queue_block()")?;
                            tracing::info!("fetched block {}", req.0);
                            // Send a response that fetching was successful.
                            // Ignore disconnection error.
                            let _ = send_resp.send(());
                            anyhow::Ok(())
                        }
                        .await
                        .with_context(|| format!("get_block({})", req.0))
                    });
                }
            });

            service.run(ctx, stream).await?;
            Ok(())
        })
        .await
    }

    /// Handles an inbound stream.
    /// Closes the stream if there is another inbound stream opened from the same peer.
    #[tracing::instrument(name = "gossip::run_inbound_stream", skip_all)]
    pub(crate) async fn run_inbound_stream(
        &self,
        ctx: &ctx::Ctx,
        mut stream: noise::Stream,
    ) -> anyhow::Result<()> {
        let conn = handshake::inbound(ctx, &self.cfg, self.genesis().hash(), &mut stream).await?;
        tracing::info!("peer = {:?}", conn.key);
        self.inbound.insert(conn.key.clone(), conn.clone()).await?;
        let res = self.run_stream(ctx, stream).await;
        self.inbound.remove(&conn.key).await;
        res
    }

    /// Connects to a peer and handles the resulting stream.
    #[tracing::instrument(name = "gossip::run_outbound_stream", skip_all)]
    pub(crate) async fn run_outbound_stream(
        &self,
        ctx: &ctx::Ctx,
        peer: &node::PublicKey,
        addr: net::Host,
    ) -> anyhow::Result<()> {
        let addr = *addr
            .resolve(ctx)
            .await?
            .context("resolve()")?
            .choose(&mut ctx.rng())
            .with_context(|| "{addr:?} resolved to empty address set")?;

        let mut stream = preface::connect(ctx, addr, preface::Endpoint::GossipNet).await?;
        let conn =
            handshake::outbound(ctx, &self.cfg, self.genesis().hash(), &mut stream, peer).await?;
        tracing::info!("peer = {peer:?}");
        self.outbound.insert(peer.clone(), conn.into()).await?;
        let res = self.run_stream(ctx, stream).await;
        self.outbound.remove(peer).await;
        res
    }
}
