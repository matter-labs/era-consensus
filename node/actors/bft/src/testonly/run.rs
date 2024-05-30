use super::{Behavior, Node};
use network::{io, Config};
use rand::prelude::SliceRandom;
use std::collections::{HashMap, HashSet};
use tracing::Instrument as _;
use zksync_concurrency::{
    ctx::{
        self,
        channel::{UnboundedReceiver, UnboundedSender},
    },
    oneshot, scope,
};
use zksync_consensus_network as network;
use zksync_consensus_roles::validator::{self, Genesis, PublicKey};
use zksync_consensus_storage::testonly::new_store;
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
        genesis: &Genesis,
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
                let want = honest[0].block(ctx, i).await?;
                for store in &honest[1..] {
                    assert_eq!(want, store.block(ctx, i).await?);
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

        for (i, spec) in specs.iter().enumerate() {
            let (actor_pipe, dispatcher_pipe) = pipe::new();
            let validator_key = spec.net.validator_key.as_ref().unwrap().public();
            let port = spec.net.server_addr.port();

            validator_ports.entry(validator_key).or_default().push(port);

            sends.insert(port, actor_pipe.send);
            recvs.push((port, actor_pipe.recv));

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
        // Taking these refeferences is necessary for the `scope::run!` environment lifetime rules to compile
        // with `async move`, which in turn is necessary otherwise it the spawned process could not borrow `port`.
        // Potentially `ctx::NoCopy` could be used with `port`.
        let sends = &sends;
        let validator_ports = &validator_ports;

        // Run networks by receiving from all consensus instances:
        // * identify the view they are in from the message
        // * identify the partition they are in based on their network id
        // * either broadcast to all other instances in the partition, or find out the network
        //   identity of the target validator and send to it _iff_ they are in the same partition
        scope::run!(ctx, |ctx, s| async move {
            for (port, recv) in recvs {
                s.spawn(async move {
                    twins_receive_loop(ctx, port, recv, sends, validator_ports, splits).await
                });
            }
            anyhow::Ok(())
        })
        .await
    })
    .await
}

/// Receive input messages from the consensus actor and send them to the others
/// according to the partition schedule of the port associated with this instance.
async fn twins_receive_loop(
    ctx: &ctx::Ctx,
    port: Port,
    mut recv: UnboundedReceiver<io::InputMessage>,
    sends: &HashMap<Port, UnboundedSender<io::OutputMessage>>,
    validator_ports: &HashMap<PublicKey, Vec<Port>>,
    splits: &PortSplitSchedule,
) -> anyhow::Result<()> {
    let rng = &mut ctx.rng();
    let start = &ctx.now();

    // We need to buffer messages that cannot be delivered due to partitioning, and deliver them later.
    // The spec says that the network is expected to deliver messages eventually, potentially out of order,
    // caveated by the fact that the actual implementation only keep retrying the last message.
    // However with the actors instantiated in by this test, that would not be sufficient because
    // there is no gossip network here, and if a replica misses a proposal, it won't get it via gossip,
    // and will forever be unable to finalize blocks.
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

        if can_send {
            let s = &sends[&target_port];

            // Messages can be delivered in arbitrary order.
            stash.shuffle(rng);

            for unstashed in stash.drain(0..) {
                let view = output_msg_view_number(&unstashed);
                let kind = output_msg_label(&unstashed);
                eprintln!("   ^^^ unstashed view={view} from={port} to={target_port} kind={kind}");
                s.send(unstashed);
            }
            eprintln!("   >>> sending view={view} from={port} to={target_port} kind={kind}");
            s.send(msg);
            // TODO: If the message is a LeaderPrepare then remember the block sent,
            //       then if the next message is a LeaderCommit with a CommitQC then
            //       prepare a FinalBlock and pretend that we have a gossip layer by
            //       by calling block_store.queue_block on every other node.
        } else {
            eprintln!("   VVV stashed view={view} from={port} to={target_port} kind={kind}");
            stash.push(msg)
        }
    };

    while let Ok(io::InputMessage::Consensus(message)) = recv.recv(ctx).await {
        let view_number = message.message.msg.view().number.0 as usize;
        // Here we assume that all instances start from view 0 in the tests.
        // If the view is higher than what we have planned for, assume no partitions.
        // Every node is guaranteed to be present in only one partition.
        let partitions_opt = splits.get(view_number);

        let msg = || {
            io::OutputMessage::Consensus(io::ConsensusReq {
                msg: message.message.clone(),
                ack: oneshot::channel().0,
            })
        };

        match message.recipient {
            io::Target::Broadcast => match partitions_opt {
                None => {
                    eprintln!("broadcasting view={view_number} from={port} target=all");
                    for target_port in sends.keys() {
                        send_or_stash(true, *target_port, msg());
                    }
                }
                Some(ps) => {
                    for p in ps {
                        let can_send = p.contains(&port);
                        eprintln!("broadcasting view={view_number} from={port} target={p:?} can_send={can_send} t={}", start.elapsed().as_seconds_f64());
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
                            eprintln!(
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
                                    eprintln!("unicasting view={view_number} from={port} target={target_port } can_send={can_send} t={}", start.elapsed().as_seconds_f64());
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

fn output_msg_view_number(msg: &io::OutputMessage) -> u64 {
    match msg {
        io::OutputMessage::Consensus(cr) => cr.msg.msg.view().number.0,
    }
}

fn output_msg_label(msg: &io::OutputMessage) -> &str {
    match msg {
        io::OutputMessage::Consensus(cr) => cr.msg.msg.label(),
    }
}
