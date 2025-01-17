use super::{Behavior, Node};
use crate::{FromNetworkMessage, ToNetworkMessage};
use anyhow::Context as _;
use network::Config;
use rand::seq::SliceRandom;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tracing::Instrument as _;
use zksync_concurrency::{
    ctx::{
        self,
        channel::{self, UnboundedReceiver, UnboundedSender},
    },
    oneshot, scope, sync,
};
use zksync_consensus_network::{self as network};
use zksync_consensus_roles::{validator, validator::testonly::Setup};
use zksync_consensus_storage::{testonly::TestMemoryStorage, BlockStore};

pub(crate) enum Network {
    Real,
    Twins(PortRouter),
}

/// Number of phases within a view for which we consider different partitions.
///
/// Technically there are 4 phases but that results in tests timing out as
/// the chance of a reaching consensus in any round goes down rapidly.
///
/// Instead we can just use two phase-partitions: one for the LeaderProposal,
/// and another for everything else. This models the typical adversarial
/// scenario of not everyone getting the proposal.
pub(crate) const NUM_PHASES: usize = 2;

/// Index of the phase in which the message appears, to decide which partitioning to apply.
fn msg_phase_number(msg: &validator::ConsensusMsg) -> usize {
    use validator::ConsensusMsg;
    let phase = match msg {
        ConsensusMsg::LeaderProposal(_) => 0,
        ConsensusMsg::ReplicaCommit(_) => 1,
        ConsensusMsg::ReplicaTimeout(_) => 1,
        ConsensusMsg::ReplicaNewView(_) => 1,
    };
    assert!(phase < NUM_PHASES);
    phase
}

/// Identify different network identities of twins by their listener port.
/// They are all expected to be on localhost, but `ListenerAddr` can't be
/// directly used as a map key.
pub(crate) type Port = u16;
/// A partition consists of ports that can communicate.
pub(crate) type PortPartition = HashSet<Port>;
/// A split is a list of disjunct partitions.
pub(crate) type PortSplit = Vec<PortPartition>;
/// A schedule contains a list of splits (one for each phase) for every view.
pub(crate) type PortSplitSchedule = Vec<[PortSplit; NUM_PHASES]>;
/// Function to decide whether a message can go from a source to a target port.
pub(crate) type PortRouterFn = dyn Fn(&validator::ConsensusMsg, Port, Port) -> Option<bool> + Sync;

/// A predicate to govern who can communicate to whom a given message.
pub(crate) enum PortRouter {
    /// List of port splits for each view/phase, where ports in the same partition can send any message to each other.
    Splits(PortSplitSchedule),
    /// Custom routing function which can take closer control of which message can be sent in which direction,
    /// in order to reenact particular edge cases.
    #[allow(dead_code)]
    Custom(Box<PortRouterFn>),
}

impl PortRouter {
    /// Decide whether a message can be sent from a sender to a target in the given view/phase.
    ///
    /// Returning `None` means the there was no more routing data and the test can decide to
    /// allow all communication or to abort a runaway test.
    fn can_send(&self, msg: &validator::ConsensusMsg, from: Port, to: Port) -> Option<bool> {
        match self {
            PortRouter::Splits(splits) => {
                // Here we assume that all instances start from view 0 in the tests.
                // If the view is higher than what we have planned for, assume no partitions.
                // Every node is guaranteed to be present in only one partition.
                let view_number = msg.view().number.0 as usize;
                let phase_number = msg_phase_number(msg);
                splits
                    .get(view_number)
                    .and_then(|ps| ps.get(phase_number))
                    .map(|partitions| {
                        partitions
                            .iter()
                            .any(|p| p.contains(&from) && p.contains(&to))
                    })
            }
            PortRouter::Custom(f) => f(msg, from, to),
        }
    }
}

/// Config for the test. Determines the parameters to run the test with.
pub(crate) struct Test {
    pub(crate) network: Network,
    pub(crate) nodes: Vec<(Behavior, u64)>,
    pub(crate) blocks_to_finalize: usize,
}

#[derive(thiserror::Error, Debug)]
pub(crate) enum TestError {
    #[error("finalized conflicting blocks")]
    BlockConflict,
    #[error(transparent)]
    Other(#[from] ctx::Error),
}

impl From<anyhow::Error> for TestError {
    fn from(err: anyhow::Error) -> Self {
        Self::Other(err.into())
    }
}

impl From<ctx::Canceled> for TestError {
    fn from(err: ctx::Canceled) -> Self {
        Self::Other(err.into())
    }
}

impl Test {
    /// Run a test with the given parameters and a random network setup.
    pub(crate) async fn run(&self, ctx: &ctx::Ctx) -> Result<(), TestError> {
        let rng = &mut ctx.rng();
        let setup = Setup::new_with_weights(rng, self.nodes.iter().map(|(_, w)| *w).collect());
        let nets: Vec<_> = network::testonly::new_configs(rng, &setup, 1);
        self.run_with_config(ctx, nets, &setup).await
    }

    /// Run a test with the given parameters and network configuration.
    pub(crate) async fn run_with_config(
        &self,
        ctx: &ctx::Ctx,
        nets: Vec<Config>,
        setup: &Setup,
    ) -> Result<(), TestError> {
        let mut nodes = vec![];
        let mut honest = vec![];
        scope::run!(ctx, |ctx, s| async {
            for (i, net) in nets.into_iter().enumerate() {
                let store = TestMemoryStorage::new(ctx, setup).await;
                s.spawn_bg(async { Ok(store.runner.run(ctx).await?) });

                if self.nodes[i].0 == Behavior::Honest {
                    honest.push(store.blocks.clone());
                }

                nodes.push(Node {
                    net,
                    behavior: self.nodes[i].0,
                    block_store: store.blocks,
                });
            }
            assert!(!honest.is_empty());
            s.spawn_bg(async { Ok(run_nodes(ctx, &self.network, &nodes).await?) });

            // Run the nodes until all honest nodes store enough finalized blocks.
            assert!(self.blocks_to_finalize > 0);
            let first = setup.genesis.first_block;
            let last = first + (self.blocks_to_finalize as u64 - 1);
            for store in &honest {
                store.wait_until_queued(ctx, last).await?;
            }

            // Check that the stored blocks are consistent.
            for i in 0..self.blocks_to_finalize as u64 {
                let i = first + i;
                // Only comparing the payload; the justification might be different.
                let want = honest[0].block(ctx, i).await?.context("missing block")?;
                for store in &honest[1..] {
                    let got = store.block(ctx, i).await?.context("missing block")?;
                    if want.payload() != got.payload() {
                        return Err(TestError::BlockConflict);
                    }
                }
            }
            Ok::<_, TestError>(())
        })
        .await
    }
}

/// Run a set of nodes.
async fn run_nodes(ctx: &ctx::Ctx, network: &Network, specs: &[Node]) -> anyhow::Result<()> {
    match network {
        Network::Real => run_nodes_real(ctx, specs).await,
        Network::Twins(router) => run_nodes_twins(ctx, specs, router).await,
    }
}

/// Run a set of nodes with a real network.
async fn run_nodes_real(ctx: &ctx::Ctx, specs: &[Node]) -> anyhow::Result<()> {
    scope::run!(ctx, |ctx, s| async {
        let mut nodes = vec![];
        for (i, spec) in specs.iter().enumerate() {
            let (send, recv) = sync::prunable_mpsc::channel(
                crate::inbound_filter_predicate,
                crate::inbound_selection_function,
            );
            let (node, runner) = network::testonly::Instance::new_with_channel(
                spec.net.clone(),
                spec.block_store.clone(),
                send,
                recv,
            );
            s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node", i)));
            nodes.push(node);
        }
        network::testonly::instant_network(ctx, nodes.iter()).await?;
        for (i, node) in nodes.into_iter().enumerate() {
            let spec = &specs[i];
            s.spawn(
                async {
                    let node = node;
                    spec.run(ctx, node.consensus_receiver, node.consensus_sender)
                        .await
                }
                .instrument(tracing::info_span!("node", i)),
            );
        }
        Ok(())
    })
    .await
}

/// Run a set of nodes with a Twins network configuration.
async fn run_nodes_twins(
    ctx: &ctx::Ctx,
    specs: &[Node],
    router: &PortRouter,
) -> anyhow::Result<()> {
    scope::run!(ctx, |ctx, s| async {
        // All known network ports of a validator, so that we can tell if any of
        // those addresses are in the same partition as the sender.
        let mut validator_ports: HashMap<_, Vec<Port>> = HashMap::new();
        // Outbox of consensus instances, paired with their network identity,
        // so we can tell which partition they are in in the given view.
        let mut recvs = vec![];
        // Inbox of the consensus instances, indexed by their network identity,
        // so that we can send to the one which is in the same partition as the sender.
        let mut sends = HashMap::new();
        // Blockstores of nodes, indexed by port; used to simulate the effect of gossip.
        let mut stores = HashMap::new();
        // Outbound gossip relationships to simulate the effects of block fetching.
        let mut gossip_targets = HashMap::new();

        for (i, spec) in specs.iter().enumerate() {
            let (output_channel_send, output_channel_recv) = channel::unbounded();
            let (input_channel_send, input_channel_recv) = crate::create_input_channel();

            let validator_key = spec.net.validator_key.as_ref().unwrap().public();
            let port = spec.net.server_addr.port();

            validator_ports.entry(validator_key).or_default().push(port);

            sends.insert(port, input_channel_send);
            recvs.push((port, output_channel_recv));
            stores.insert(port, spec.block_store.clone());
            gossip_targets.insert(
                port,
                spec.net
                    .gossip
                    .static_outbound
                    .values()
                    .map(|host| {
                        let addr: std::net::SocketAddr = host.0.parse().expect("valid address");
                        addr.port()
                    })
                    .collect(),
            );

            // Run consensus node.
            s.spawn(
                async { spec.run(ctx, input_channel_recv, output_channel_send).await }
                    .instrument(tracing::info_span!("node", i, port)),
            );
        }
        // Taking these references is necessary for the `scope::run!` environment lifetime rules to compile
        // with `async move`, which in turn is necessary otherwise it the spawned process could not borrow `port`.
        // Potentially `ctx::NoCopy` could be used with `port`.
        let sends = &sends;
        let stores = &stores;
        let gossip_targets = &gossip_targets;
        let (gossip_send, gossip_recv) = channel::unbounded();

        // Run networks by receiving from all consensus instances:
        // * identify the view they are in from the message
        // * identify the partition they are in based on their network id
        // * either broadcast to all other instances in the partition, or find out the network
        //   identity of the target validator and send to it iff they are in the same partition
        // * simulate the gossiping of finalized blocks.
        scope::run!(ctx, |ctx, s| async move {
            for (i, (port, recv)) in recvs.into_iter().enumerate() {
                let gossip_send = gossip_send.clone();
                s.spawn(async move {
                    twins_receive_loop(
                        ctx,
                        router,
                        sends,
                        TwinsGossipConfig {
                            targets: &gossip_targets[&port],
                            send: gossip_send,
                        },
                        port,
                        recv,
                    )
                    .instrument(tracing::info_span!("node", i, port))
                    .await
                });
            }
            s.spawn(async { twins_gossip_loop(ctx, stores, gossip_recv).await });
            anyhow::Ok(())
        })
        .await
    })
    .await
}

/// Receive input messages from the consensus component and send them to the others
/// according to the partition schedule of the port associated with this instance.
///
/// We have to simulate the gossip layer which isn't instantiated by these tests.
/// If we don't, then if a replica misses a LeaderProposal message it won't ever get the payload
/// and won't be able to finalize the block, and won't participate further in the consensus.
async fn twins_receive_loop(
    ctx: &ctx::Ctx,
    router: &PortRouter,
    sends: &HashMap<Port, sync::prunable_mpsc::Sender<FromNetworkMessage>>,
    gossip: TwinsGossipConfig<'_>,
    port: Port,
    mut recv: UnboundedReceiver<ToNetworkMessage>,
) -> anyhow::Result<()> {
    let rng = &mut ctx.rng();

    // Finalized block number iff this node can gossip to the target and the message contains a QC.
    let block_to_gossip = |target_port: Port, msg: &FromNetworkMessage| {
        if !gossip.targets.contains(&target_port) {
            return None;
        }
        output_msg_commit_qc(msg).map(|qc| qc.header().number)
    };

    // We need to buffer messages that cannot be delivered due to partitioning, and deliver them later.
    // The spec says that the network is expected to deliver messages eventually, potentially out of order,
    // caveated by the fact that the actual implementation only keeps retrying the last message.
    // A separate issue is the definition of "later", without actually adding timing assumptions:
    // * If we want to allow partitions which don't have enough replicas for a quorum, and the replicas
    //   don't move on from a view until they reach quorum, then "later" could be defined by so many
    //   messages in a view that it indicates a timeout loop. NB the consensus might not actually have
    //   an actual timeout loop and might rely on the network trying to re-broadcast forever.
    // * OTOH if all partitions are at least quorum sized, then we there will be a subset of nodes that
    //   can move on to the next view, in which a new partition configuration will allow them to broadcast
    //   to previously isolated peers.
    // * One idea is to wait until replica A wants to send to replica B in a view when they are no longer
    //   partitioned, and then unstash all previous A-to-B messages.
    // * If that wouldn't be acceptable then we could have some kind of global view of stashed messages
    //   and unstash them as soon as someone moves on to a new view.
    let mut stashes: HashMap<Port, Vec<FromNetworkMessage>> = HashMap::new();

    // Either stash a message, or unstash all messages and send them to the target along with the new one.
    let mut send_or_stash = |can_send: bool, target_port: Port, msg: FromNetworkMessage| {
        let stash = stashes.entry(target_port).or_default();
        let view = output_msg_view_number(&msg);
        let kind = output_msg_label(&msg);

        // Remove any previously stashed message of the same kind, because the network will only
        // try to send the last one of each, not all pending messages.
        stash.retain(|stashed| output_msg_label(stashed) != kind);

        if !can_send {
            tracing::info!("   VVV stashed view={view} from={port} to={target_port} kind={kind}");
            stash.push(msg);
            return;
        }

        let s = &sends[&target_port];

        // Send after taking note of potentially gossipable blocks.
        let send = |msg| {
            if let Some(number) = block_to_gossip(target_port, &msg) {
                tracing::info!("   ~~~ gossip from={port} to={target_port} number={number}");
                gossip.send.send(TwinsGossipMessage {
                    from: port,
                    to: target_port,
                    number,
                });
            }
            s.send(msg);
        };

        // Messages can be delivered in arbitrary order.
        stash.shuffle(rng);

        for unstashed in stash.drain(0..) {
            let view = output_msg_view_number(&unstashed);
            let kind = output_msg_label(&unstashed);
            tracing::info!("   ^^^ unstashed view={view} from={port} to={target_port} kind={kind}");
            send(unstashed);
        }
        tracing::info!("   >>> sending view={view} from={port} to={target_port} kind={kind}");
        send(msg);
    };

    while let Ok(message) = recv.recv(ctx).await {
        let view = message.message.msg.view().number.0 as usize;
        let kind = message.message.msg.label();

        let msg = || FromNetworkMessage {
            msg: message.message.clone(),
            ack: oneshot::channel().0,
        };

        let can_send = |to| {
            match router.can_send(&message.message.msg, port, to) {
                Some(can_send) => Ok(can_send),
                None => anyhow::bail!("ran out of port schedule; we probably wouldn't finalize blocks even if we continued")
            }
        };

        tracing::info!("broadcasting view={view} from={port} kind={kind}");
        for target_port in sends.keys() {
            send_or_stash(can_send(*target_port)?, *target_port, msg());
        }
    }

    Ok(())
}

/// Simulate the effects of gossip: if the source node can gossip to the target,
/// and the message being sent contains a CommitQC, and the sender has the
/// referenced finalized block in its store, then assume that the target
/// could fetch this block if they wanted, and insert it directly into their store.
///
/// This happens concurrently with the actual message passing, so it's just an
/// approximation. It also happens out of order. This method only contains the
/// send loop, to deal with the spawning of store operations.
async fn twins_gossip_loop(
    ctx: &ctx::Ctx,
    stores: &HashMap<Port, Arc<BlockStore>>,
    mut recv: UnboundedReceiver<TwinsGossipMessage>,
) -> anyhow::Result<()> {
    scope::run!(ctx, |ctx, s| async move {
        while let Ok(TwinsGossipMessage {
            from,
            to,
            mut number,
        }) = recv.recv(ctx).await
        {
            let local_store = &stores[&from];
            let remote_store = &stores[&to];
            let first_needed = remote_store.queued().next();

            loop {
                // Stop going back if the target already has the block.
                if number < first_needed {
                    break;
                }
                // Perform the storage operations asynchronously because `queue_block` will
                // wait for all dependencies to be inserted first.
                s.spawn_bg(async move {
                    // Wait for the source to actually have the block.
                    if local_store.wait_until_queued(ctx, number).await.is_err() {
                        return Ok(());
                    }
                    let Ok(Some(block)) = local_store.block(ctx, number).await else {
                        tracing::info!(
                            "   ~~x gossip unavailable from={from} to={to} number={number}"
                        );
                        return Ok(());
                    };
                    tracing::info!("   ~~> gossip queue from={from} to={to} number={number}");
                    let _ = remote_store.queue_block(ctx, block).await;
                    tracing::info!("   ~~V gossip stored from={from} to={to} number={number}");
                    Ok(())
                });
                // Be pessimistic and try to insert all ancestors, to minimise the chance that
                // for some reason a node doesn't get a finalized block from anyone.
                // Without this some scenarios with twins actually fail.
                if let Some(prev) = number.prev() {
                    number = prev;
                } else {
                    break;
                };
            }
        }
        Ok(())
    })
    .await
}

fn output_msg_view_number(msg: &FromNetworkMessage) -> validator::ViewNumber {
    msg.msg.msg.view().number
}

fn output_msg_label(msg: &FromNetworkMessage) -> &str {
    msg.msg.msg.label()
}

fn output_msg_commit_qc(msg: &FromNetworkMessage) -> Option<&validator::CommitQC> {
    use validator::ConsensusMsg;

    let justification = match &msg.msg.msg {
        ConsensusMsg::ReplicaTimeout(msg) => return msg.high_qc.as_ref(),
        ConsensusMsg::ReplicaCommit(_) => return None,
        ConsensusMsg::ReplicaNewView(msg) => &msg.justification,
        ConsensusMsg::LeaderProposal(msg) => &msg.justification,
    };

    match justification {
        validator::ProposalJustification::Commit(commit_qc) => Some(commit_qc),
        validator::ProposalJustification::Timeout(timeout_qc) => timeout_qc.high_qc(),
    }
}

struct TwinsGossipMessage {
    from: Port,
    to: Port,
    number: validator::BlockNumber,
}

struct TwinsGossipConfig<'a> {
    /// Ports to which this node should gossip to.
    targets: &'a HashSet<Port>,
    /// Channel over which to send gossip messages.
    send: UnboundedSender<TwinsGossipMessage>,
}
