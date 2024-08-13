//! Network actor maintaining a pool of outbound and inbound connections to other nodes.
use anyhow::Context as _;
use gossip::attestation;
use std::sync::Arc;
use tracing::Instrument as _;
use zksync_concurrency::{
    ctx::{self, channel},
    error::Wrap as _,
    limiter, scope,
};
use zksync_consensus_storage::{BatchStore, BlockStore};
use zksync_consensus_utils::pipe::ActorPipe;

mod config;
pub mod consensus;
mod frame;
pub mod gossip;
pub mod http;
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
pub use metrics::MeteredStreamStats;

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
    /// Network state.
    net: Arc<Network>,
    /// Receiver of the messages from the dispatcher.
    receiver: channel::UnboundedReceiver<io::InputMessage>,
}

impl Network {
    /// Constructs a new network actor state.
    /// Call `run_network` to run the actor.
    pub fn new(
        cfg: Config,
        block_store: Arc<BlockStore>,
        batch_store: Arc<BatchStore>,
        pipe: ActorPipe<io::InputMessage, io::OutputMessage>,
        attestation: Arc<attestation::Controller>,
    ) -> (Arc<Self>, Runner) {
        let gossip =
            gossip::Network::new(cfg, block_store, batch_store, pipe.send, attestation);
        let consensus = consensus::Network::new(gossip.clone());
        let net = Arc::new(Self { gossip, consensus });
        (
            net.clone(),
            Runner {
                net,
                receiver: pipe.recv,
            },
        )
    }

    /// Registers metrics for this state.
    pub fn register_metrics(self: &Arc<Self>) {
        metrics::NetworkGauges::register(Arc::downgrade(self));
    }

    /// Handles a dispatcher message.
    async fn handle_message(
        &self,
        _ctx: &ctx::Ctx,
        message: io::InputMessage,
    ) -> anyhow::Result<()> {
        match message {
            io::InputMessage::Consensus(message) => {
                self.consensus
                    .as_ref()
                    .context("not a validator node")?
                    .msg_pool
                    .send(Arc::new(message));
            }
        }
        Ok(())
    }
}

impl Runner {
    /// Runs the network actor.
    pub async fn run(mut self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        let res: ctx::Result<()> = scope::run!(ctx, |ctx, s| async {
            let mut listener = self
                .net
                .gossip
                .cfg
                .server_addr
                .bind()
                .context("server_addr.bind()")?;

            // Handle incoming messages.
            s.spawn(async {
                // We don't propagate cancellation errors
                while let Ok(message) = self.receiver.recv(ctx).await {
                    s.spawn(async {
                        if let Err(err) = self.net.handle_message(ctx, message).await {
                            tracing::info!("handle_message(): {err:#}");
                        }
                        Ok(())
                    });
                }
                Ok(())
            });

            // Fetch missing blocks in the background.
            s.spawn(async {
                self.net.gossip.run_block_fetcher(ctx).await;
                Ok(())
            });

            // Fetch missing batches in the background.
            s.spawn(async {
                self.net.gossip.run_batch_fetcher(ctx).await;
                Ok(())
            });

            // Maintain static gossip connections.
            for (peer, addr) in &self.net.gossip.cfg.gossip.static_outbound {
                s.spawn::<()>(async {
                    let addr = &*addr;
                    loop {
                        let res = self
                            .net
                            .gossip
                            .run_outbound_stream(ctx, peer, addr.clone())
                            .await;
                        if let Err(err) = res {
                            tracing::info!("gossip.run_outbound_stream({addr:?}): {err:#}");
                        }
                        ctx.sleep(CONNECT_RETRY).await?;
                    }
                });
            }

            if let Some(c) = &self.net.consensus {
                let validators = &c.gossip.genesis().validators;
                // If we are active validator ...
                if validators.contains(&c.key.public()) {
                    // Maintain outbound connections.
                    for peer in validators.keys() {
                        s.spawn(async {
                            c.maintain_connection(ctx, peer).await;
                            Ok(())
                        });
                    }
                }
            }

            // TODO: check if we are active attester to get new L1 Batches, sign them and broadcast the signature

            let accept_limiter = limiter::Limiter::new(ctx, self.net.gossip.cfg.tcp_accept_rate);
            loop {
                accept_limiter.acquire(ctx, 1).await?;
                let stream = metrics::MeteredStream::accept(ctx, &mut listener)
                    .await
                    .wrap("accept()")?;
                s.spawn(async {
                    // May fail if the socket got closed.
                    let addr = stream.peer_addr().context("peer_addr()")?;
                    let res = async {
                        tracing::info!("new connection");
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
                                self.net
                                    .gossip
                                    .run_inbound_stream(ctx, stream)
                                    .await
                                    .context("gossip.run_inbound_stream()")?;
                            }
                        }
                        anyhow::Ok(())
                    }
                    .instrument(tracing::info_span!("accept_connection", ?addr))
                    .await;
                    if let Err(err) = res {
                        tracing::info!("{addr}: {err:#}");
                    }
                    Ok(())
                });
            }
        })
        .await;
        match res {
            Ok(()) | Err(ctx::Error::Canceled(_)) => Ok(()),
            Err(ctx::Error::Internal(err)) => Err(err),
        }
    }
}
