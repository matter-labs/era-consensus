//! Network actor maintaining a pool of outbound and inbound connections to other nodes.
use anyhow::Context as _;
use std::sync::Arc;
use zksync_concurrency::{ctx, ctx::channel, scope, time};
use zksync_consensus_storage::BlockStore;
use zksync_consensus_utils::pipe::ActorPipe;

pub mod consensus;
mod config;
mod event;
mod frame;
pub mod gossip;
pub mod io;
mod metrics;
mod mux;
mod noise;
mod pool;
mod preface;
pub mod proto;
mod rpc;
mod state;
pub mod testonly;
#[cfg(test)]
mod tests;
mod watch;

pub use config::*;


/// State of the network actor observable outside of the actor.
pub struct Network {
    /// Consensus network state.
    pub(crate) consensus: Option<Arc<consensus::Network>>,
    /// Gossip network state.
    pub(crate) gossip: Arc<gossip::Network>,
}

/// Runner of the Network background tasks. 
#[must_use]
pub struct Runner {
    net: Arc<Network>,
    receiver: channel::UnboundedReceiver<io::InputMessage>,
}

impl Network {
    /// Constructs a new network actor state.
    /// Call `run_network` to run the actor.
    pub fn new(
        ctx: &ctx::Ctx,
        cfg: Config,
        block_store: Arc<BlockStore>,
        pipe: ActorPipe<io::InputMessage, io::OutputMessage>,
    ) -> (Arc<Self>,Runner) {
        let gossip = gossip::Network::new(cfg, block_store, pipe.send);
        let consensus = consensus::Network::new(ctx,gossip.clone());
        let net = Arc::new(Self { gossip, consensus });
        (net.clone(), Runner {
            net,
            receiver: pipe.recv,
        })
    }

    /// Registers metrics for this state.
    pub fn register_metrics(self: &Arc<Self>) {
        metrics::NetworkGauges::register(Arc::downgrade(self));
    }

    async fn handle_message(&self, ctx: &ctx::Ctx, message: io::InputMessage) -> anyhow::Result<()> {
        const CONSENSUS_MSG_TIMEOUT: time::Duration = time::Duration::seconds(10);
        const GET_BLOCK_TIMEOUT: time::Duration = time::Duration::seconds(10);

        match message {
            io::InputMessage::Consensus(message) => {
                if let Some(consensus) = &self.consensus {
                    let ctx = &ctx.with_timeout(CONSENSUS_MSG_TIMEOUT);
                    match message.recipient {
                        io::Target::Validator(key) => consensus.send(ctx,&key,message.message).await?,
                        io::Target::Broadcast => consensus.broadcast(ctx,message.message).await?,
                    }
                }
            }
            io::InputMessage::SyncBlocks(io::SyncBlocksInputMessage::GetBlock {
                recipient, number, response,
            }) => {
                let ctx = &ctx.with_timeout(GET_BLOCK_TIMEOUT);
                let _ = response.send(match self.gossip.get_block(ctx,&recipient,number).await {
                    Ok(Some(block)) => Ok(block),
                    Ok(None) => Err(io::GetBlockError::NotAvailable),
                    Err(err) => Err(io::GetBlockError::Internal(err)),
                });
            }
        }
        Ok(())
    }
}

impl Runner {
    /// Runs the network actor.
    pub async fn run(mut self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        let mut listener = self.net.gossip.cfg.server_addr.bind()?;

        scope::run!(ctx, |ctx, s| async {
            // Handle incoming messages.
            s.spawn(async {
                // We don't propagate cancellation errors
                while let Ok(message) = self.receiver.recv(ctx).await {
                    s.spawn(async {
                        if let Err(err) = self.net.handle_message(ctx,message).await {
                            tracing::info!("handle_message(): {err:#}");
                        }
                        Ok(())
                    });
                }
                Ok(())
            });

            // Maintain static gossip connections. 
            for (peer, addr) in &self.net.gossip.cfg.gossip.static_outbound {
                s.spawn::<()>(async {
                    loop {
                        let run_result = self.net.gossip.run_outbound_stream(ctx, peer, *addr).await;
                        if let Err(err) = run_result {
                            tracing::info!("gossip.run_outbound_stream(): {err:#}");
                        }
                        ctx.sleep(CONNECT_RETRY).await?;
                    }
                });
            }

            if let Some(c) = &self.net.consensus {
                // If we are active validator ...
                if c.gossip.cfg.genesis.validators.contains(&c.key.public()) {
                    // Maintain outbound connections.
                    for peer in c.clients.keys() {
                        s.spawn(c.maintain_connection(ctx,peer));
                    }
                    // Announce IP periodically.
                    s.spawn(async {
                        c.run_address_announcer(ctx).await;
                        Ok(())
                    });
                }
            }

            // TODO(gprusak): add rate limit and inflight limit for inbound handshakes.
            while let Ok(stream) = metrics::MeteredStream::listen(ctx, &mut listener).await {
                let stream = stream.context("listener.accept()")?;
                s.spawn(async {
                    let res = async {
                        let (stream, endpoint) = preface::accept(ctx, stream)
                            .await
                            .context("preface::accept()")?;
                        match endpoint {
                            preface::Endpoint::ConsensusNet => {
                                if let Some(c) = &self.net.consensus {
                                    c.run_inbound_stream(ctx, stream)
                                        .await
                                        .context("consensus.run_inbound_stream()")?;
                                }
                            }
                            preface::Endpoint::GossipNet => {
                                self.net.gossip.run_inbound_stream(ctx, stream)
                                    .await
                                    .context("gossip.run_inbound_stream()")?;
                            }
                        }
                        anyhow::Ok(())
                    }
                    .await;
                    if let Err(err) = res {
                        tracing::info!("{err:#}");
                    }
                    Ok(())
                });
            }
            Ok(())
        })
        .await
    }
}
