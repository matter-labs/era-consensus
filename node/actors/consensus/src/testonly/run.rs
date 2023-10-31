use super::{Behavior, Metrics, Node};
use crate::{testonly, Consensus};
use anyhow::Context;
use concurrency::{ctx, ctx::channel, oneshot, scope, signal};
use roles::validator;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use storage::{FallbackReplicaStateStore, InMemoryStorage};
use tracing::Instrument as _;
use utils::pipe;

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
        let nodes: Vec<_> = network::testonly::Instance::new(rng, self.nodes.len(), 1)
            .into_iter()
            .enumerate()
            .map(|(i, net)| Node {
                net,
                behavior: self.nodes[i],
            })
            .collect();
        scope::run!(ctx, |ctx, s| async {
            let (metrics_send, mut metrics_recv) = channel::unbounded();
            s.spawn_bg(run_nodes(ctx, self.network, &nodes, metrics_send));

            // Get only the honest replicas.
            let honest: HashSet<_> = nodes
                .iter()
                .filter(|node| node.behavior == Behavior::Honest)
                .map(|node| node.net.consensus_config().key.public())
                .collect();
            assert!(!honest.is_empty());

            let mut finalized: HashMap<validator::BlockNumber, validator::BlockHeaderHash> =
                HashMap::new();
            let mut observers: HashMap<validator::BlockNumber, HashSet<validator::PublicKey>> =
                HashMap::new();
            let mut fully_observed = 0;

            while fully_observed < self.blocks_to_finalize {
                let metric = metrics_recv.recv(ctx).await?;
                if !honest.contains(&metric.validator) {
                    continue;
                }
                let block = metric.finalized_block;
                let hash = block.header.hash();
                assert_eq!(*finalized.entry(block.header.number).or_insert(hash), hash);
                let observers = observers.entry(block.header.number).or_default();
                if observers.insert(metric.validator.clone()) && observers.len() == honest.len() {
                    fully_observed += 1;
                }
            }
            Ok(())
        })
        .await
    }
}

/// Run a set of nodes.
async fn run_nodes(
    ctx: &ctx::Ctx,
    network: Network,
    nodes: &[Node],
    metrics: channel::UnboundedSender<Metrics>,
) -> anyhow::Result<()> {
    let keys: Vec<_> = nodes
        .iter()
        .map(|node| node.net.consensus_config().key.clone())
        .collect();
    let (genesis_block, _) = testonly::make_genesis(&keys, validator::Payload(vec![]));
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
                    let storage = InMemoryStorage::new(genesis_block.clone());
                    let storage = FallbackReplicaStateStore::from_store(Arc::new(storage));

                    let consensus = Consensus::new(
                        ctx,
                        consensus_actor_pipe,
                        node.net.consensus_config().key.clone(),
                        validator_set,
                        storage,
                    )
                    .await
                    .context("consensus")?;

                    scope::run!(ctx, |ctx, s| async {
                        network_ready.recv(ctx).await?;
                        s.spawn_blocking(|| consensus.run(ctx).context("consensus.run()"));
                        node.run_executor(ctx, consensus_pipe, network_pipe, metrics.clone())
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
                        let mut recv = recv;
                        use network::io;

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
