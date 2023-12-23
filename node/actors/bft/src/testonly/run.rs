use super::{Behavior, Node};
use crate::{testonly, Config};
use anyhow::Context;
use std::{collections::HashMap, sync::Arc};
use tracing::Instrument as _;
use zksync_concurrency::{ctx, oneshot, sync, scope, signal};
use zksync_consensus_network as network;
use zksync_consensus_roles::validator;
use zksync_consensus_storage::{testonly::in_memory, BlockStore};
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
    pub(crate) nodes: Vec<Behavior>,
    pub(crate) blocks_to_finalize: usize,
}

impl Test {
    /// Run a test with the given parameters.
    pub(crate) async fn run(&self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        let rng = &mut ctx.rng();
        let nets: Vec<_> = network::testonly::Instance::new(rng, self.nodes.len(), 1);
        let keys: Vec<_> = nets
            .iter()
            .map(|node| node.consensus_config().key.clone())
            .collect();
        let (genesis_block, _) =
            testonly::make_genesis(&keys, validator::Payload(vec![]), validator::BlockNumber(0));
        let mut nodes = vec![];
        for (i, net) in nets.into_iter().enumerate() {
            let block_store = Box::new(in_memory::BlockStore::new(genesis_block.clone()));
            let block_store = Arc::new(BlockStore::new(ctx, block_store, 10).await?);
            nodes.push(Node {
                net,
                behavior: self.nodes[i],
                block_store,
            });
        }

        // Get only the honest replicas.
        let honest: Vec<_> = nodes
            .iter()
            .filter(|node| node.behavior == Behavior::Honest)
            .collect();
        assert!(!honest.is_empty());

        // Run the nodes until all honest nodes store enough finalized blocks.
        scope::run!(ctx, |ctx, s| async {
            s.spawn_bg(run_nodes(ctx, self.network, &nodes));
            let want_block = validator::BlockNumber(self.blocks_to_finalize as u64);
            for n in &honest {
                sync::wait_for(ctx, &mut n.block_store.subscribe(), |state| state.next() > want_block).await?;
            }
            Ok(())
        })
        .await?;

        // Check that the stored blocks are consistent.
        for i in 0..self.blocks_to_finalize as u64 + 1 {
            let i = validator::BlockNumber(i);
            let want = honest[0].block_store.block(ctx, i).await?;
            for n in &honest[1..] {
                assert_eq!(want, n.block_store.block(ctx, i).await?);
            }
        }
        Ok(())
    }
}

/// Run a set of nodes.
async fn run_nodes(ctx: &ctx::Ctx, network: Network, nodes: &[Node]) -> anyhow::Result<()> {
    let network_ready = signal::Once::new();
    let mut network_pipes = HashMap::new();
    let mut network_send = HashMap::new();
    let mut network_recv = HashMap::new();
    scope::run!(ctx, |ctx, s| async {
        for (i, node) in nodes.iter().enumerate() {
            let consensus_config = node.net.consensus_config();
            let validator_key = consensus_config.key.clone();
            let validator_set = node.net.to_config().validators;

            let (consensus_actor_pipe, consensus_pipe) = pipe::new();
            let (network_actor_pipe, network_pipe) = pipe::new();
            network_pipes.insert(validator_key.public(), network_actor_pipe);
            s.spawn(
                async {
                    scope::run!(ctx, |ctx, s| async {
                        network_ready.recv(ctx).await?;
                        s.spawn(node.block_store.run_background_tasks(ctx));
                        s.spawn(async {
                            Config {
                                secret_key: validator_key,
                                validator_set,
                                block_store: node.block_store.clone(),
                                replica_store: Box::new(in_memory::ReplicaStore::default()),
                                payload_manager: node.behavior.payload_manager(),
                            }
                            .run(ctx, consensus_actor_pipe)
                            .await
                            .context("consensus.run()")
                        });
                        node.run_executor(ctx, consensus_pipe, network_pipe)
                            .await
                            .context("executor.run()")
                    })
                    .await
                }
                .instrument(tracing::info_span!("node", i)),
            );
        }
        match network {
            Network::Real => {
                for (i, node) in nodes.iter().enumerate() {
                    let state = node.net.state().clone();
                    let pipe = network_pipes
                        .remove(&node.net.consensus_config().key.public())
                        .unwrap();
                    s.spawn(
                        async {
                            network::run_network(ctx, state, pipe)
                                .await
                                .context("network")
                        }
                        .instrument(tracing::info_span!("node", i)),
                    );
                }
                network::testonly::instant_network(ctx, nodes.iter().map(|n| &n.net)).await?;
                network_ready.send();
                Ok(())
            }
            Network::Mock => {
                for (key, pipe) in network_pipes {
                    network_send.insert(key.clone(), pipe.send);
                    network_recv.insert(key.clone(), pipe.recv);
                }
                for (_, recv) in network_recv {
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
                                io::Target::Validator(v) => {
                                    network_send.get(&v).unwrap().send(msg())
                                }
                                io::Target::Broadcast => {
                                    network_send.values().for_each(|s| s.send(msg()))
                                }
                            }
                        }
                        Ok(())
                    });
                }
                network_ready.send();
                Ok(())
            }
        }
    })
    .await
}
