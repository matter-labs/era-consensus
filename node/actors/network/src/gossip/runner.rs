use super::{batch_votes::BatchVotes, handshake, Network, ValidatorAddrs};
use crate::{noise, preface, rpc};
use anyhow::Context as _;
use async_trait::async_trait;
use rand::seq::SliceRandom;
use std::sync::atomic::Ordering;
use zksync_concurrency::{ctx, net, scope, sync};
use zksync_consensus_roles::{attester::BatchNumber, node};
use zksync_consensus_storage::{BatchStore, BatchStoreState, BlockStore, BlockStoreState};
use zksync_protobuf::kB;

struct PushValidatorAddrsServer<'a>(&'a Network);

#[async_trait]
impl rpc::Handler<rpc::push_validator_addrs::Rpc> for PushValidatorAddrsServer<'_> {
    fn max_req_size(&self) -> usize {
        100 * kB
    }
    async fn handle(
        &self,
        _ctx: &ctx::Ctx,
        req: rpc::push_validator_addrs::Req,
    ) -> anyhow::Result<()> {
        self.0
            .push_validator_addrs_calls
            .fetch_add(1, Ordering::SeqCst);
        self.0
            .validator_addrs
            .update(&self.0.genesis().validators, &req.0)
            .await?;
        Ok(())
    }
}

/// Receive the snapshot of known batch votes from a remote peer.
///
/// The server receives the *diff* from remote peers, which it merges into the common register.
struct PushBatchVotesServer<'a>(&'a Network);

#[async_trait::async_trait]
impl rpc::Handler<rpc::push_batch_votes::Rpc> for PushBatchVotesServer<'_> {
    /// Here we bound the buffering of incoming batch messages.
    fn max_req_size(&self) -> usize {
        100 * kB
    }

    async fn handle(&self, _ctx: &ctx::Ctx, req: rpc::push_batch_votes::Req) -> anyhow::Result<()> {
        self.0
            .batch_votes
            .update(
                self.0.genesis().attesters.as_ref().context("attesters")?,
                &req.0,
            )
            .await?;
        Ok(())
    }
}

/// Represents what we know about the state of available blocks on the remote peer.
struct PushBlockStoreStateServer<'a> {
    state: sync::watch::Sender<BlockStoreState>,
    /// The network is required for the verification of messages.
    net: &'a Network,
}

impl<'a> PushBlockStoreStateServer<'a> {
    fn new(net: &'a Network) -> Self {
        Self {
            state: sync::watch::channel(BlockStoreState {
                first: net.genesis().first_block,
                last: None,
            })
            .0,
            net,
        }
    }
}

/// Represents what we know about the state of available batches on the remote peer.
struct PushBatchStoreStateServer {
    state: sync::watch::Sender<BatchStoreState>,
    max_batch_size: usize,
}

impl PushBatchStoreStateServer {
    /// Start out not knowing anything about the remote peer.
    fn new(max_batch_size: usize) -> Self {
        Self {
            state: sync::watch::channel(BatchStoreState {
                first: BatchNumber(0),
                last: None,
            })
            .0,
            max_batch_size,
        }
    }
}

#[async_trait]
impl rpc::Handler<rpc::push_block_store_state::Rpc> for &PushBlockStoreStateServer<'_> {
    fn max_req_size(&self) -> usize {
        10 * kB
    }
    async fn handle(
        &self,
        _ctx: &ctx::Ctx,
        req: rpc::push_block_store_state::Req,
    ) -> anyhow::Result<()> {
        req.0.verify(self.net.genesis())?;
        self.state.send_replace(req.0);
        Ok(())
    }
}

#[async_trait]
impl rpc::Handler<rpc::push_batch_store_state::Rpc> for &PushBatchStoreStateServer {
    fn max_req_size(&self) -> usize {
        self.max_batch_size.saturating_add(kB)
    }
    async fn handle(
        &self,
        _ctx: &ctx::Ctx,
        req: rpc::push_batch_store_state::Req,
    ) -> anyhow::Result<()> {
        req.0.verify()?;
        self.state.send_replace(req.0);
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
        Ok(rpc::get_block::Resp(self.block(ctx, req.0).await?))
    }
}

#[async_trait]
impl rpc::Handler<rpc::get_batch::Rpc> for &BatchStore {
    fn max_req_size(&self) -> usize {
        kB
    }
    async fn handle(
        &self,
        ctx: &ctx::Ctx,
        req: rpc::get_batch::Req,
    ) -> anyhow::Result<rpc::get_batch::Resp> {
        Ok(rpc::get_batch::Resp(self.batch(ctx, req.0).await?))
    }
}

impl Network {
    /// Manages lifecycle of a single connection.
    async fn run_stream(&self, ctx: &ctx::Ctx, stream: noise::Stream) -> anyhow::Result<()> {
        let push_validator_addrs_client = rpc::Client::<rpc::push_validator_addrs::Rpc>::new(
            ctx,
            self.cfg.rpc.push_validator_addrs_rate,
        );
        let push_validator_addrs_server = PushValidatorAddrsServer(self);
        let push_block_store_state_client = rpc::Client::<rpc::push_block_store_state::Rpc>::new(
            ctx,
            self.cfg.rpc.push_block_store_state_rate,
        );
        let push_block_store_state_server = PushBlockStoreStateServer::new(self);
        let get_block_client =
            rpc::Client::<rpc::get_block::Rpc>::new(ctx, self.cfg.rpc.get_block_rate);
        let get_batch_client =
            rpc::Client::<rpc::get_batch::Rpc>::new(ctx, self.cfg.rpc.get_batch_rate);
        let push_batch_store_state_client = rpc::Client::<rpc::push_batch_store_state::Rpc>::new(
            ctx,
            self.cfg.rpc.push_batch_store_state_rate,
        );
        let push_batch_store_state_server = PushBatchStoreStateServer::new(self.cfg.max_batch_size);
        scope::run!(ctx, |ctx, s| async {
            let mut service = rpc::Service::new()
                .add_client(&push_validator_addrs_client)
                .add_server(
                    ctx,
                    push_validator_addrs_server,
                    self.cfg.rpc.push_validator_addrs_rate,
                )
                .add_client(&push_block_store_state_client)
                .add_server(
                    ctx,
                    &push_block_store_state_server,
                    self.cfg.rpc.push_block_store_state_rate,
                )
                .add_client(&get_block_client)
                .add_server(ctx, &*self.block_store, self.cfg.rpc.get_block_rate)
                .add_client(&get_batch_client)
                .add_server(ctx, &*self.batch_store, self.cfg.rpc.get_batch_rate)
                .add_client(&push_batch_store_state_client)
                .add_server(
                    ctx,
                    &push_batch_store_state_server,
                    self.cfg.rpc.get_batch_rate,
                )
                .add_server(ctx, rpc::ping::Server, rpc::ping::RATE);

            // If there is an attester committee then
            if self.genesis().attesters.as_ref().is_some() {
                let push_signature_client = rpc::Client::<rpc::push_batch_votes::Rpc>::new(
                    ctx,
                    self.cfg.rpc.push_batch_votes_rate,
                );
                let push_signature_server = PushBatchVotesServer(self);
                service = service.add_client(&push_signature_client).add_server(
                    ctx,
                    push_signature_server,
                    self.cfg.rpc.push_batch_votes_rate,
                );
                // Push L1 batch votes updates to peer.
                s.spawn::<()>(async {
                    let push_signature_client = push_signature_client;
                    // Snapshot of the batches when we last pushed to the peer.
                    let mut old = BatchVotes::default();
                    // Subscribe to what we know about the state of the whole network.
                    let mut sub = self.batch_votes.subscribe();
                    sub.mark_changed();
                    loop {
                        let new = sync::changed(ctx, &mut sub).await?.clone();
                        // Get the *new* votes, which haven't been pushed before.
                        let diff = new.get_newer(&old);
                        if diff.is_empty() {
                            continue;
                        }
                        old = new;
                        let req = rpc::push_batch_votes::Req(diff);
                        push_signature_client.call(ctx, &req, kB).await?;
                    }
                });
            }

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
                    let req = rpc::push_block_store_state::Req(state.clone());
                    push_block_store_state_client.call(ctx, &req, kB).await?;
                    state = self.block_store.wait_for_queued_change(ctx, &state).await?;
                }
            });

            // Push batch store state updates to peer.
            s.spawn::<()>(async {
                let mut state = self.batch_store.queued();
                loop {
                    let req = rpc::push_batch_store_state::Req(state.clone());
                    push_batch_store_state_client.call(ctx, &req, kB).await?;
                    state = self
                        .batch_store
                        .wait_until_queued(ctx, state.next())
                        .await?;
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
                let state = &mut push_block_store_state_server.state.subscribe();
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
                            let block = call
                                .call(ctx, &req, self.cfg.max_block_size.saturating_add(kB))
                                .await?
                                .0
                                .context("empty response")?;
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

            // Perform get_batch calls to peer.
            s.spawn::<()>(async {
                let state = &mut push_batch_store_state_server.state.subscribe();
                loop {
                    let call = get_batch_client.reserve(ctx).await?;
                    let (req, send_resp) = self.fetch_queue.accept_batch(ctx, state).await?;
                    let req = rpc::get_batch::Req(req);
                    s.spawn(async {
                        let req = req;
                        // Failing to fetch a batch causes a disconnect:
                        // - peer predeclares which batches are available and race condition
                        //   with batch pruning should be very rare, so we can consider
                        //   an empty response to be offending
                        // - a stream for the call has been already reserved,
                        //   so the peer is expected to answer immediately. The timeout
                        //   should be high enough to accommodate network hiccups
                        // - a disconnect is not a ban, so the peer is free to try to
                        //   reconnect.
                        async {
                            let ctx_with_timeout =
                                self.cfg.rpc.get_batch_timeout.map(|t| ctx.with_timeout(t));
                            let ctx = ctx_with_timeout.as_ref().unwrap_or(ctx);
                            let batch = call
                                .call(ctx, &req, self.cfg.max_batch_size.saturating_add(kB))
                                .await?
                                .0
                                .context("empty response")?;
                            anyhow::ensure!(batch.number == req.0, "received wrong batch");
                            // Storing the batch will fail in case batch is invalid.
                            self.batch_store
                                .queue_batch(ctx, batch, self.genesis().clone())
                                .await
                                .context("queue_batch()")?;
                            tracing::info!("fetched batch {}", req.0);
                            // Send a response that fetching was successful.
                            // Ignore disconnection error.
                            let _ = send_resp.send(());
                            anyhow::Ok(())
                        }
                        .await
                        .with_context(|| format!("get_batch({})", req.0))
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
    #[tracing::instrument(level = "info", name = "gossip", skip_all)]
    pub(crate) async fn run_inbound_stream(
        &self,
        ctx: &ctx::Ctx,
        mut stream: noise::Stream,
    ) -> anyhow::Result<()> {
        let peer =
            handshake::inbound(ctx, &self.cfg.gossip, self.genesis().hash(), &mut stream).await?;
        tracing::info!("peer = {peer:?}");
        self.inbound
            .insert(peer.clone(), stream.get_values())
            .await?;
        let res = self.run_stream(ctx, stream).await;
        self.inbound.remove(&peer).await;
        res
    }

    /// Connects to a peer and handles the resulting stream.
    #[tracing::instrument(level = "info", name = "gossip", skip_all)]
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
        handshake::outbound(
            ctx,
            &self.cfg.gossip,
            self.genesis().hash(),
            &mut stream,
            peer,
        )
        .await?;
        tracing::info!("peer = {peer:?}");
        self.outbound
            .insert(peer.clone(), stream.get_values())
            .await?;
        let res = self.run_stream(ctx, stream).await;
        self.outbound.remove(peer).await;
        res
    }
}
