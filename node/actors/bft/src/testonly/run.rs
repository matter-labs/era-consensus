use super::{Behavior, Node};
use anyhow::bail;
use network::{io, Config};
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
    oneshot, scope,
};
use zksync_consensus_network as network;
use zksync_consensus_roles::validator;
use zksync_consensus_storage::{testonly::new_store, BlockStore};
use zksync_consensus_utils::pipe;

#[derive(Clone)]
pub(crate) enum Network {
    Real,
    Mock,
    Twins(PortSplitSchedule),
}

// Identify different network identities of twins by their listener port.
// They are all expected to be on localhost, but `ListenerAddr` can't be
// directly used as a map key.
pub(crate) type Port = u16;
pub(crate) type PortPartition = HashSet<Port>;
pub(crate) type PortSplit = Vec<PortPartition>;
pub(crate) type PortSplitSchedule = Vec<PortSplit>;

/// Config for the test. Determines the parameters to run the test with.
#[derive(Clone)]
pub(crate) struct Test {
    pub(crate) network: Network,
    pub(crate) nodes: Vec<(Behavior, u64)>,
    pub(crate) blocks_to_finalize: usize,
}

impl Test {
    /// Run a test with the given parameters and a random network setup.
    pub(crate) async fn run(&self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        let rng = &mut ctx.rng();
        let setup = validator::testonly::Setup::new_with_weights(
            rng,
            self.nodes.iter().map(|(_, w)| *w).collect(),
        );
        let nets: Vec<_> = network::testonly::new_configs(rng, &setup, 1);
        self.run_with_config(ctx, nets, &setup.genesis).await
    }

    /// Run a test with the given parameters and network configuration.
    pub(crate) async fn run_with_config(
        &self,
        ctx: &ctx::Ctx,
        nets: Vec<Config>,
        genesis: &validator::Genesis,
    ) -> anyhow::Result<()> {
        let mut nodes = vec![];
        let mut honest = vec![];
        scope::run!(ctx, |ctx, s| async {
            for (i, net) in nets.into_iter().enumerate() {
                let (store, runner) = new_store(ctx, genesis).await;
                s.spawn_bg(runner.run(ctx));
                if self.nodes[i].0 == Behavior::Honest {
                    honest.push(store.clone());
                }
                nodes.push(Node {
                    net,
                    behavior: self.nodes[i].0,
                    block_store: store,
                });
            }
            assert!(!honest.is_empty());
            s.spawn_bg(run_nodes(ctx, &self.network, &nodes));

            // Run the nodes until all honest nodes store enough finalized blocks.
            assert!(self.blocks_to_finalize > 0);
            let first = genesis.first_block;
            let last = first + (self.blocks_to_finalize as u64 - 1);
            for store in &honest {
                store.wait_until_queued(ctx, last).await?;
            }

            // Check that the stored blocks are consistent.
            for i in 0..self.blocks_to_finalize as u64 {
                let i = first + i;
                // Only comparing the payload; the justification might be different.
                let want = honest[0]
                    .block(ctx, i)
                    .await?
                    .expect("checked its existence")
                    .payload;
                for store in &honest[1..] {
                    assert_eq!(want, store.block(ctx, i).await?.unwrap().payload);
                }
            }
            Ok(())
        })
        .await
    }
}

/// Run a set of nodes.
async fn run_nodes(ctx: &ctx::Ctx, network: &Network, specs: &[Node]) -> anyhow::Result<()> {
    match network {
        Network::Real => run_nodes_real(ctx, specs).await,
        Network::Mock => run_nodes_mock(ctx, specs).await,
        Network::Twins(splits) => run_nodes_twins(ctx, specs, splits).await,
    }
}

/// Run a set of nodes with a real network.
async fn run_nodes_real(ctx: &ctx::Ctx, specs: &[Node]) -> anyhow::Result<()> {
    scope::run!(ctx, |ctx, s| async {
        let mut nodes = vec![];
        for (i, spec) in specs.iter().enumerate() {
            let (node, runner) =
                network::testonly::Instance::new(spec.net.clone(), spec.block_store.clone());
            s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node", i)));
            nodes.push(node);
        }
        network::testonly::instant_network(ctx, nodes.iter()).await?;
        for (i, node) in nodes.into_iter().enumerate() {
            let spec = &specs[i];
            s.spawn(
                async {
                    let mut node = node;
                    spec.run(ctx, node.pipe()).await
                }
                .instrument(tracing::info_span!("node", i)),
            );
        }
        Ok(())
    })
    .await
}

/// Run a set of nodes with a mock network.
async fn run_nodes_mock(ctx: &ctx::Ctx, specs: &[Node]) -> anyhow::Result<()> {
    scope::run!(ctx, |ctx, s| async {
        // Actor inputs, ie. where the test can send messages to the consensus.
        let mut sends = HashMap::new();
        // Actor outputs, ie. the messages the actor wants to send to the others.
        let mut recvs = vec![];
        for (i, spec) in specs.iter().enumerate() {
            let (actor_pipe, dispatcher_pipe) = pipe::new();
            let key = spec.net.validator_key.as_ref().unwrap().public();
            sends.insert(key, actor_pipe.send);
            recvs.push(actor_pipe.recv);
            // Run consensus; the dispatcher pipe is its network connection, which means we can use the actor pipe to:
            // * send Output messages from other actors to this consensus instance
            // * receive Input messages sent by this consensus to the other actors
            s.spawn(
                async {
                    let mut network_pipe = dispatcher_pipe;
                    spec.run(ctx, &mut network_pipe).await
                }
                .instrument(tracing::info_span!("node", i)),
            );
        }
        // Run mock networks by receiving the output-turned-input from all consensus
        // instances and forwarding them to the others.
        scope::run!(ctx, |ctx, s| async {
            for recv in recvs {
                s.spawn(async {
                    use zksync_consensus_network::io;
                    let mut recv = recv;
                    while let Ok(io::InputMessage::Consensus(message)) = recv.recv(ctx).await {
                        let msg = || {
                            io::OutputMessage::Consensus(io::ConsensusReq {
                                msg: message.message.clone(),
                                ack: oneshot::channel().0,
                            })
                        };
                        match message.recipient {
                            io::Target::Validator(v) => sends.get(&v).unwrap().send(msg()),
                            io::Target::Broadcast => sends.values().for_each(|s| s.send(msg())),
                        }
                    }
                    Ok(())
                });
            }
            anyhow::Ok(())
        })
        .await
    })
    .await
}

/// Run a set of nodes with a Twins network configuration.
async fn run_nodes_twins(
    ctx: &ctx::Ctx,
    specs: &[Node],
    splits: &PortSplitSchedule,
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
            let (actor_pipe, dispatcher_pipe) = pipe::new();
            let validator_key = spec.net.validator_key.as_ref().unwrap().public();
            let port = spec.net.server_addr.port();

            validator_ports.entry(validator_key).or_default().push(port);

            sends.insert(port, actor_pipe.send);
            recvs.push((port, actor_pipe.recv));
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

            // Run consensus; the dispatcher pipe is its network connection, which means we can use the actor pipe to:
            // * send Output messages from other actors to this consensus instance
            // * receive Input messages sent by this consensus to the other actors
            s.spawn(
                async {
                    let mut network_pipe = dispatcher_pipe;
                    spec.run(ctx, &mut network_pipe).await
                }
                .instrument(tracing::info_span!("node", i)),
            );
        }
        // Taking these references is necessary for the `scope::run!` environment lifetime rules to compile
        // with `async move`, which in turn is necessary otherwise it the spawned process could not borrow `port`.
        // Potentially `ctx::NoCopy` could be used with `port`.
        let validator_ports = &validator_ports;
        let sends = &sends;
        let stores = &stores;
        let gossip_targets = &gossip_targets;
        let (gossip_send, gossip_recv) = channel::unbounded();

        // Run networks by receiving from all consensus instances:
        // * identify the view they are in from the message
        // * identify the partition they are in based on their network id
        // * either broadcast to all other instances in the partition, or find out the network
        //   identity of the target validator and send to it iff they are in the same partition
        // * simulate the gossiping of finalized blockss
        scope::run!(ctx, |ctx, s| async move {
            for (port, recv) in recvs {
                let gossip_send = gossip_send.clone();
                s.spawn(async move {
                    twins_receive_loop(
                        ctx,
                        splits,
                        validator_ports,
                        sends,
                        TwinsGossipConfig {
                            targets: &gossip_targets[&port],
                            send: gossip_send,
                        },
                        port,
                        recv,
                    )
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

/// Receive input messages from the consensus actor and send them to the others
/// according to the partition schedule of the port associated with this instance.
///
/// We have to simulate the gossip layer which isn't instantiated by these tests.
/// If we don't, then if a replica misses a LeaderPrepare message it won't ever get the payload
/// and won't be able to finalize the block, and won't participate further in the consensus.
async fn twins_receive_loop(
    ctx: &ctx::Ctx,
    splits: &PortSplitSchedule,
    validator_ports: &HashMap<validator::PublicKey, Vec<Port>>,
    sends: &HashMap<Port, UnboundedSender<io::OutputMessage>>,
    gossip: TwinsGossipConfig<'_>,
    port: Port,
    mut recv: UnboundedReceiver<io::InputMessage>,
) -> anyhow::Result<()> {
    let rng = &mut ctx.rng();
    let start = std::time::Instant::now(); // Just to give an idea of how long time passes between rounds.

    // Finalized block number iff this node can gossip to the target and the message contains a QC.
    let block_to_gossip = |target_port: Port, msg: &io::OutputMessage| {
        if !gossip.targets.contains(&target_port) {
            return None;
        }
        output_msg_commit_qc(msg).map(|qc| qc.header().number)
    };

    // We need to buffer messages that cannot be delivered due to partitioning, and deliver them later.
    // The spec says that the network is expected to deliver messages eventually, potentially out of order,
    // caveated by the fact that the actual implementation only keeps retrying the last message..
    // A separate issue is the definition of "later", without actually adding timing assumptions:
    // * If we want to allow partitions which don't have enough replicas for a quorum, and the replicas
    //   don't move on from a view until they reach quorum, then "later" could be defined by so many
    //   messages in a view that it indicates a timeout loop. NB the consensus might not actually have
    //   an actual timeout loop and might rely on the network trying to re-broadcast forever.
    // * OTOH if all partitions are at least quorum sized, then we there will be a subset of nodes that
    //   can move on to the next view, in which a new partition configuration will allow them to broadcast
    //   to previously isolated peers.
    // * One idea is to wait until replica A wants to send to replica B in a view when they are no longer
    //   partitioned, and then unstash all previous A-to-B messages. This would _not_ work with HotStuff
    //   out of the box, because replicas only communicate with their leader, so if for example B missed
    //   a LeaderCommit from A in an earlier view, B will not respond to the LeaderPrepare from C because
    //   they can't commit the earlier block until they get a new message from A. However since
    //   https://github.com/matter-labs/era-consensus/pull/119 the ReplicaPrepare messages are broadcasted,
    //   so we shouldn't have to wait long for A to unstash its messages to B.
    // * If that wouldn't be acceptable then we could have some kind of global view of stashed messages
    //   and unstash them as soon as someone moves on to a new view.
    let mut stashes: HashMap<Port, Vec<io::OutputMessage>> = HashMap::new();

    // Either stash a message, or unstash all messages and send them to the target along with the new one.
    let mut send_or_stash = |can_send: bool, target_port: Port, msg: io::OutputMessage| {
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

    while let Ok(io::InputMessage::Consensus(message)) = recv.recv(ctx).await {
        let view_number = message.message.msg.view().number.0 as usize;
        // Here we assume that all instances start from view 0 in the tests.
        // If the view is higher than what we have planned for, assume no partitions.
        // Every node is guaranteed to be present in only one partition.
        let partitions_opt = splits.get(view_number);

        if partitions_opt.is_none() {
            bail!(
                "ran out of scheduled rounds; most likely cannot finalize blocks even if we go on"
            );
        }

        let msg = || {
            io::OutputMessage::Consensus(io::ConsensusReq {
                msg: message.message.clone(),
                ack: oneshot::channel().0,
            })
        };

        match message.recipient {
            io::Target::Broadcast => match partitions_opt {
                None => {
                    tracing::info!("broadcasting view={view_number} from={port} target=all");
                    for target_port in sends.keys() {
                        send_or_stash(true, *target_port, msg());
                    }
                }
                Some(ps) => {
                    for p in ps {
                        let can_send = p.contains(&port);
                        tracing::info!("broadcasting view={view_number} from={port} target={p:?} can_send={can_send} t={}", start.elapsed().as_secs());
                        for target_port in p {
                            send_or_stash(can_send, *target_port, msg());
                        }
                    }
                }
            },
            io::Target::Validator(v) => {
                let target_ports = &validator_ports[&v];

                match partitions_opt {
                    None => {
                        for target_port in target_ports {
                            tracing::info!(
                                "unicasting view={view_number} from={port} target={target_port}"
                            );
                            send_or_stash(true, *target_port, msg());
                        }
                    }
                    Some(ps) => {
                        for p in ps {
                            let can_send = p.contains(&port);
                            for target_port in target_ports {
                                if p.contains(target_port) {
                                    tracing::info!("unicasting view={view_number} from={port} target={target_port } can_send={can_send} t={}", start.elapsed().as_secs());
                                    send_or_stash(can_send, *target_port, msg());
                                }
                            }
                        }
                    }
                }
            }
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
                // Stop if the source doesn't actually have this block to give.
                let Ok(Some(block)) = local_store.block(ctx, number).await else {
                    break;
                };
                // Perform the storing operation asynchronously because `queue_block` will
                // wait for all dependencies to be inserted first.
                s.spawn_bg(async move {
                    let _ = remote_store.queue_block(ctx, block).await;
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

fn output_msg_view_number(msg: &io::OutputMessage) -> validator::ViewNumber {
    match msg {
        io::OutputMessage::Consensus(cr) => cr.msg.msg.view().number,
    }
}

fn output_msg_label(msg: &io::OutputMessage) -> &str {
    match msg {
        io::OutputMessage::Consensus(cr) => cr.msg.msg.label(),
    }
}

fn output_msg_commit_qc(msg: &io::OutputMessage) -> Option<&validator::CommitQC> {
    use validator::ConsensusMsg;
    match msg {
        io::OutputMessage::Consensus(cr) => match &cr.msg.msg {
            ConsensusMsg::ReplicaPrepare(rp) => rp.high_qc.as_ref(),
            ConsensusMsg::LeaderPrepare(lp) => lp.justification.high_qc(),
            ConsensusMsg::ReplicaCommit(_) => None,
            ConsensusMsg::LeaderCommit(lc) => Some(&lc.justification),
        },
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
