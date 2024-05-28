use super::{Behavior, Node};
use network::Config;
use std::collections::HashMap;
use tracing::Instrument as _;
use zksync_concurrency::{ctx, oneshot, scope};
use zksync_consensus_network as network;
use zksync_consensus_roles::validator::{self, Genesis};
use zksync_consensus_storage::testonly::new_store;
use zksync_consensus_utils::pipe;

#[derive(Clone, Copy)]
pub(crate) enum Network {
    Real,
    Mock,
}

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
            s.spawn_bg(run_nodes(ctx, self.network, &nodes));

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
async fn run_nodes(ctx: &ctx::Ctx, network: Network, specs: &[Node]) -> anyhow::Result<()> {
    match network {
        Network::Real => run_nodes_real(ctx, specs).await,
        Network::Mock => run_nodes_mock(ctx, specs).await,
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
            // Run consensus; the dispatcher pipe is its network connection,
            // which means we can use the actor pipe to:
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
