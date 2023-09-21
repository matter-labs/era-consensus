//! Network actor maintaining a pool of outbound and inbound connections to other nodes.
use super::{consensus, event::Event, gossip, preface};
use crate::io::{InputMessage, OutputMessage, SyncState};
use anyhow::Context as _;
use concurrency::{ctx, ctx::channel, metrics, net, scope, sync::watch};
use std::sync::Arc;
use utils::pipe::ActorPipe;

/// Network actor config.
#[derive(Clone)]
pub struct Config {
    /// TCP socket address to listen for inbound connections at.
    pub server_addr: net::tcp::ListenerAddr,
    /// Gossip network config.
    pub gossip: gossip::Config,
    /// Consensus network config.
    pub consensus: consensus::Config,
}

/// State of the network actor observable outside of the actor.
pub struct State {
    /// Network configuration.
    pub(crate) cfg: Config,
    /// Consensus network state.
    pub(crate) consensus: consensus::State,
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
    ) -> Arc<Self> {
        Arc::new(Self {
            gossip: gossip::State::new(&cfg.gossip, sync_state),
            consensus: consensus::State::new(&cfg.consensus),
            events,
            cfg,
        })
    }

    /// Config getter.
    pub fn cfg(&self) -> &Config {
        &self.cfg
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

        s.spawn(async {
            consensus::run_client(ctx, state.as_ref(), consensus_recv)
                .await
                .context("consensus::run_client")
        });

        // TODO(gprusak): add rate limit and inflight limit for inbound handshakes.
        while let Ok(stream) = net::tcp::accept(ctx, &mut listener).await {
            let stream = stream.context("listener.accept()")?;
            s.spawn(async {
                let res = async {
                    let (stream, endpoint) = preface::accept(ctx, stream).await?;
                    match endpoint {
                        preface::Endpoint::ConsensusNet => {
                            consensus::run_inbound_stream(ctx, &state, &pipe.send, stream)
                                .await
                                .context("consensus::run_inbound_stream()")
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

impl State {
    /// Collection of gauges monitoring the state,
    /// which can be added to prometheus registry.
    pub fn collector(self: &Arc<Self>) -> metrics::Collector<Self> {
        metrics::Collector::new(Arc::downgrade(self))
            .gauge(
                "network_gossip_inbound_connections",
                "number of active gossipnet inbound connections",
                |s| s.gossip.inbound.subscribe().borrow().current().len() as f64,
            )
            .gauge(
                "network_gossip_outbound_connections",
                "number of active gossipnet outbound connections",
                |s| s.gossip.outbound.subscribe().borrow().current().len() as f64,
            )
            .gauge(
                "network_consensus_inbound_connections",
                "number of active consensusnet inbound connections",
                |s| s.consensus.inbound.subscribe().borrow().current().len() as f64,
            )
            .gauge(
                "network_consensus_outbound_connections",
                "number of active consensusnet outbound connections",
                |s| s.consensus.outbound.subscribe().borrow().current().len() as f64,
            )
    }
}
