//! Network actor maintaining a pool of outbound and inbound connections to other nodes.
use super::{consensus, event::Event, gossip, metrics, preface};
use crate::io::{InputMessage, OutputMessage, SyncState};
use anyhow::Context as _;
use std::sync::Arc;
use zksync_concurrency::{ctx, ctx::channel, net, scope, sync::watch};
use zksync_consensus_roles::validator;
use zksync_consensus_utils::pipe::ActorPipe;

/// Network actor config.
#[derive(Debug, Clone)]
pub struct Config {
    /// TCP socket address to listen for inbound connections at.
    pub server_addr: net::tcp::ListenerAddr,
    /// Validators which
    /// - client should establish outbound connections to.
    /// - server should accept inbound connections from (1 per validator).
    pub validators: validator::ValidatorSet,
    /// Gossip network config.
    pub gossip: gossip::Config,
    /// Consensus network config. If not present, the node will not participate in the consensus network.
    pub consensus: Option<consensus::Config>,
}

/// Part of configuration shared among network modules.
#[derive(Debug)]
pub(crate) struct SharedConfig {
    /// TCP socket address to listen for inbound connections at.
    pub(crate) server_addr: net::tcp::ListenerAddr,
    /// Validators which
    /// - client should establish outbound connections to.
    /// - server should accept inbound connections from (1 per validator).
    pub(crate) validators: validator::ValidatorSet,
}

/// State of the network actor observable outside of the actor.
pub struct State {
    /// Configuration shared among network modules.
    pub(crate) cfg: SharedConfig,
    /// Consensus network state.
    pub(crate) consensus: Option<consensus::State>,
    /// Gossip network state.
    pub(crate) gossip: gossip::State,

    /// TESTONLY: channel of network events which the tests can observe.
    // TODO(gprusak): consider if it would be enough to make it pub(crate).
    pub(crate) events: Option<channel::UnboundedSender<Event>>,
}

impl State {
    /// Constructs a new network actor state.
    /// Call `run_network` to run the actor.
    pub fn new(
        cfg: Config,
        events: Option<channel::UnboundedSender<Event>>,
        sync_state: Option<watch::Receiver<SyncState>>,
    ) -> anyhow::Result<Arc<Self>> {
        let consensus = cfg
            .consensus
            .map(|consensus_cfg| consensus::State::new(consensus_cfg, &cfg.validators))
            .transpose()?;
        let this = Self {
            gossip: gossip::State::new(cfg.gossip, sync_state),
            consensus,
            events,
            cfg: SharedConfig {
                server_addr: cfg.server_addr,
                validators: cfg.validators,
            },
        };
        Ok(Arc::new(this))
    }

    /// Registers metrics for this state.
    pub fn register_metrics(self: &Arc<Self>) {
        metrics::NetworkGauges::register(Arc::downgrade(self));
    }
}

/// Runs the network actor.
/// WARNING: it is a bug to call multiple times in parallel
/// run_network with the same `state` argument.
/// TODO(gprusak): consider a "runnable" wrapper of `State`
/// which will be consumed by `run_network`. This way we
/// could prevent the bug above.
pub async fn run_network(
    ctx: &ctx::Ctx,
    state: Arc<State>,
    mut pipe: ActorPipe<InputMessage, OutputMessage>,
) -> anyhow::Result<()> {
    let mut listener = state.cfg.server_addr.bind()?;
    let (consensus_send, consensus_recv) = channel::unbounded();
    let (gossip_send, gossip_recv) = channel::unbounded();

    scope::run!(ctx, |ctx, s| async {
        s.spawn(async {
            // We don't propagate cancellation errors
            while let Ok(message) = pipe.recv.recv(ctx).await {
                match message {
                    InputMessage::Consensus(message) => {
                        consensus_send.send(message);
                    }
                    InputMessage::SyncBlocks(message) => {
                        gossip_send.send(message);
                    }
                }
            }
            Ok(())
        });

        s.spawn(async {
            gossip::run_client(ctx, state.as_ref(), &pipe.send, gossip_recv)
                .await
                .context("gossip::run_client")
        });

        if let Some(consensus_state) = &state.consensus {
            s.spawn(async {
                consensus::run_client(ctx, consensus_state, state.as_ref(), consensus_recv)
                    .await
                    .context("consensus::run_client")
            });
        }

        // TODO(gprusak): add rate limit and inflight limit for inbound handshakes.
        while let Ok(stream) = metrics::MeteredStream::listen(ctx, &mut listener).await {
            let stream = stream.context("listener.accept()")?;
            s.spawn(async {
                let res = async {
                    let (stream, endpoint) = preface::accept(ctx, stream).await?;
                    match endpoint {
                        preface::Endpoint::ConsensusNet => {
                            if let Some(consensus_state) = &state.consensus {
                                consensus::run_inbound_stream(
                                    ctx,
                                    consensus_state,
                                    &pipe.send,
                                    stream,
                                )
                                .await
                                .context("consensus::run_inbound_stream()")
                            } else {
                                anyhow::bail!("Node does not accept consensus network connections");
                            }
                        }
                        preface::Endpoint::GossipNet => {
                            gossip::run_inbound_stream(ctx, &state, &pipe.send, stream)
                                .await
                                .context("gossip::run_inbound_stream()")
                        }
                    }
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
