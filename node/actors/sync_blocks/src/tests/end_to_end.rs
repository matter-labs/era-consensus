//! End-to-end tests that launch a network of nodes and the `SyncBlocks` actor for each node.
use super::*;
use anyhow::Context as _;
use async_trait::async_trait;
use rand::seq::SliceRandom;
use std::fmt;
use test_casing::test_casing;
use tracing::{instrument, Instrument};
use zksync_concurrency::{
    ctx,
    ctx::channel,
    scope,
    testonly::{abort_on_panic, set_timeout},
};
use zksync_consensus_network as network;
use zksync_consensus_storage::testonly::new_store_with_first;

type NetworkDispatcherPipe =
    pipe::DispatcherPipe<network::io::InputMessage, network::io::OutputMessage>;

#[derive(Debug)]
struct Node {
    store: Arc<BlockStore>,
    start: channel::Sender<()>,
    terminate: channel::Sender<()>,
}

impl Node {
    async fn new(ctx: &ctx::Ctx, network: network::Config, setup: &Setup) -> (Self, NodeRunner) {
        Self::new_with_first(ctx, network, setup, setup.genesis.fork.first_block).await
    }

    async fn new_with_first(
        ctx: &ctx::Ctx,
        network: network::Config,
        setup: &Setup,
        first: validator::BlockNumber,
    ) -> (Self, NodeRunner) {
        let (store, store_runner) = new_store_with_first(ctx, &setup.genesis, first).await;
        let (start_send, start_recv) = channel::bounded(1);
        let (terminate_send, terminate_recv) = channel::bounded(1);

        let runner = NodeRunner {
            network,
            store: store.clone(),
            store_runner,
            start: start_recv,
            terminate: terminate_recv,
        };
        let this = Self {
            store,
            start: start_send,
            terminate: terminate_send,
        };
        (this, runner)
    }

    fn start(&self) {
        let _ = self.start.try_send(());
    }

    async fn terminate(&self, ctx: &ctx::Ctx) -> ctx::OrCanceled<()> {
        let _ = self.terminate.try_send(());
        self.terminate.closed(ctx).await
    }
}

#[must_use]
struct NodeRunner {
    network: network::Config,
    store: Arc<BlockStore>,
    store_runner: BlockStoreRunner,
    start: channel::Receiver<()>,
    terminate: channel::Receiver<()>,
}

impl NodeRunner {
    async fn run(mut self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        tracing::info!("NodeRunner::run()");
        let key = self.network.gossip.key.public();
        let (sync_blocks_actor_pipe, sync_blocks_dispatcher_pipe) = pipe::new();
        let (mut network, network_runner) =
            network::testonly::Instance::new(self.network.clone(), self.store.clone());
        let sync_blocks_config = Config::new();
        let res = scope::run!(ctx, |ctx, s| async {
            s.spawn_bg(self.store_runner.run(ctx));
            s.spawn_bg(network_runner.run(ctx));
            network.wait_for_gossip_connections().await;
            tracing::info!("Node connected to peers");

            self.start.recv(ctx).await?;
            tracing::info!("switch_on");
            s.spawn_bg(
                async {
                    Self::run_executor(ctx, sync_blocks_dispatcher_pipe, network.pipe())
                        .await
                        .with_context(|| format!("executor for {key:?}"))
                }
                .instrument(tracing::info_span!("mock_executor", ?key)),
            );
            s.spawn_bg(sync_blocks_config.run(ctx, sync_blocks_actor_pipe, self.store.clone()));
            tracing::info!("Node is fully started");

            let _ = self.terminate.recv(ctx).await;
            tracing::info!("stopping");
            Ok(())
        })
        .await;
        drop(self.terminate);
        tracing::info!("node stopped");
        res
    }

    async fn run_executor(
        ctx: &ctx::Ctx,
        mut sync_blocks_dispatcher_pipe: pipe::DispatcherPipe<InputMessage, OutputMessage>,
        network_dispatcher_pipe: &mut NetworkDispatcherPipe,
    ) -> anyhow::Result<()> {
        scope::run!(ctx, |ctx, s| async {
            s.spawn(async {
                while let Ok(message) = network_dispatcher_pipe.recv.recv(ctx).await {
                    tracing::trace!(?message, "Received network message");
                    match message {
                        network::io::OutputMessage::SyncBlocks(req) => {
                            sync_blocks_dispatcher_pipe.send.send(req.into());
                        }
                        _ => unreachable!("consensus messages should not be produced"),
                    }
                }
                Ok(())
            });

            while let Ok(message) = sync_blocks_dispatcher_pipe.recv.recv(ctx).await {
                let OutputMessage::Network(message) = message;
                tracing::trace!(?message, "Received sync blocks message");
                network_dispatcher_pipe.send.send(message.into());
            }
            Ok(())
        })
        .await
    }
}

#[async_trait]
trait GossipNetworkTest: fmt::Debug + Send {
    /// Returns the number of nodes in the gossip network and number of peers for each node.
    fn network_params(&self) -> (usize, usize);
    async fn test(self, ctx: &ctx::Ctx, setup: &Setup, network: Vec<Node>) -> anyhow::Result<()>;
}

#[instrument(level = "trace")]
async fn test_sync_blocks<T: GossipNetworkTest>(test: T) {
    abort_on_panic();
    let _guard = set_timeout(TEST_TIMEOUT);
    let ctx = &ctx::test_root(&ctx::AffineClock::new(25.));
    let rng = &mut ctx.rng();
    let (node_count, gossip_peers) = test.network_params();

    let mut setup = validator::testonly::Setup::new(rng, node_count);
    setup.push_blocks(rng, 10);
    scope::run!(ctx, |ctx, s| async {
        let mut nodes = vec![];
        for (i, net) in network::testonly::new_configs(rng, &setup, gossip_peers)
            .into_iter()
            .enumerate()
        {
            let (node, runner) = Node::new(ctx, net, &setup).await;
            s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node", i)));
            nodes.push(node);
        }
        test.test(ctx, &setup, nodes).await
    })
    .await
    .unwrap();
}

#[derive(Debug)]
struct BasicSynchronization {
    node_count: usize,
    gossip_peers: usize,
}

#[async_trait]
impl GossipNetworkTest for BasicSynchronization {
    fn network_params(&self) -> (usize, usize) {
        (self.node_count, self.gossip_peers)
    }

    async fn test(self, ctx: &ctx::Ctx, setup: &Setup, nodes: Vec<Node>) -> anyhow::Result<()> {
        let rng = &mut ctx.rng();

        tracing::info!("Check initial node states");
        for node in &nodes {
            node.start();
            let state = node.store.subscribe().borrow().clone();
            assert_eq!(state.first, setup.genesis.fork.first_block);
            assert_eq!(state.last, None);
        }

        for block in &setup.blocks[0..5] {
            let node = nodes.choose(rng).unwrap();
            node.store.queue_block(ctx, block.clone()).await.unwrap();

            tracing::info!("Wait until all nodes get block #{}", block.number());
            for node in &nodes {
                node.store.wait_until_persisted(ctx, block.number()).await?;
            }
        }

        let node = nodes.choose(rng).unwrap();
        scope::run!(ctx, |ctx, s| async {
            // Add a batch of blocks.
            for block in setup.blocks[5..].iter().rev() {
                s.spawn_bg(node.store.queue_block(ctx, block.clone()));
            }

            // Wait until nodes get all new blocks.
            let last = setup.blocks.last().unwrap().number();
            for node in &nodes {
                node.store.wait_until_persisted(ctx, last).await?;
            }
            Ok(())
        })
        .await?;
        Ok(())
    }
}

#[test_casing(5, [2, 3, 5, 7, 10])]
#[tokio::test(flavor = "multi_thread")]
async fn basic_synchronization_with_single_peer(node_count: usize) {
    test_sync_blocks(BasicSynchronization {
        node_count,
        gossip_peers: 1,
    })
    .await;
}

#[test_casing(5, [(3, 2), (5, 2), (5, 3), (7, 2), (7, 3)])]
#[tokio::test(flavor = "multi_thread")]
async fn basic_synchronization_with_multiple_peers(node_count: usize, gossip_peers: usize) {
    test_sync_blocks(BasicSynchronization {
        node_count,
        gossip_peers,
    })
    .await;
}

#[derive(Debug)]
struct SwitchingOffNodes {
    node_count: usize,
}

#[async_trait]
impl GossipNetworkTest for SwitchingOffNodes {
    fn network_params(&self) -> (usize, usize) {
        // Ensure that each node is connected to all others via an inbound or outbound channel
        (self.node_count, self.node_count / 2)
    }

    async fn test(self, ctx: &ctx::Ctx, setup: &Setup, mut nodes: Vec<Node>) -> anyhow::Result<()> {
        let rng = &mut ctx.rng();
        nodes.shuffle(rng);

        for node in &nodes {
            node.start();
        }

        for i in 0..nodes.len() {
            tracing::info!("{} nodes left", nodes.len() - i);
            let block = &setup.blocks[i];
            nodes[i..]
                .choose(rng)
                .unwrap()
                .store
                .queue_block(ctx, block.clone())
                .await
                .unwrap();
            tracing::info!("block {} inserted", block.number());

            // Wait until all remaining nodes get the new block.
            for node in &nodes[i..] {
                node.store.wait_until_persisted(ctx, block.number()).await?;
            }
            tracing::info!("All nodes received block #{}", block.number());

            // Terminate a random node.
            // We start switching off only after the first round, to make sure all nodes are fully
            // started.
            nodes[i].terminate(ctx).await.unwrap();
        }
        tracing::info!("test finished, terminating");
        Ok(())
    }
}

#[test_casing(5, 3..=7)]
#[tokio::test(flavor = "multi_thread")]
async fn switching_off_nodes(node_count: usize) {
    test_sync_blocks(SwitchingOffNodes { node_count }).await;
}

#[derive(Debug)]
struct SwitchingOnNodes {
    node_count: usize,
}

#[async_trait]
impl GossipNetworkTest for SwitchingOnNodes {
    fn network_params(&self) -> (usize, usize) {
        (self.node_count, self.node_count / 2)
    }

    async fn test(self, ctx: &ctx::Ctx, setup: &Setup, mut nodes: Vec<Node>) -> anyhow::Result<()> {
        let rng = &mut ctx.rng();
        nodes.shuffle(rng);
        for i in 0..nodes.len() {
            nodes[i].start(); // Switch on a node.
            let block = &setup.blocks[i];
            nodes[0..i + 1]
                .choose(rng)
                .unwrap()
                .store
                .queue_block(ctx, block.clone())
                .await
                .unwrap();

            // Wait until all switched on nodes get the new block.
            for node in &nodes[0..i + 1] {
                node.store.wait_until_persisted(ctx, block.number()).await?;
            }
            tracing::trace!("All nodes received block #{}", block.number());
        }
        Ok(())
    }
}

#[test_casing(5, 3..=7)]
#[tokio::test(flavor = "multi_thread")]
async fn switching_on_nodes(node_count: usize) {
    test_sync_blocks(SwitchingOnNodes { node_count }).await;
}

/// Test checking that nodes with different first block can synchronize.
#[tokio::test(flavor = "multi_thread")]
async fn test_different_first_block() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::AffineClock::new(25.));
    let rng = &mut ctx.rng();

    let mut setup = validator::testonly::Setup::new(rng, 2);
    let n = 4;
    setup.push_blocks(rng, 10);
    scope::run!(ctx, |ctx, s| async {
        let mut nodes = vec![];
        // Spawn `n` nodes, all connected to each other.
        for (i, net) in network::testonly::new_configs(rng, &setup, n)
            .into_iter()
            .enumerate()
        {
            // Choose the first block for the node at random.
            let first = setup.blocks.choose(rng).unwrap().number();
            let (node, runner) = Node::new_with_first(ctx, net, &setup, first).await;
            s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node", i)));
            node.start();
            nodes.push(node);
        }
        // Randomize the order of nodes.
        nodes.shuffle(rng);

        for block in &setup.blocks {
            // Find nodes interested in the next block.
            let interested_nodes: Vec<_> = nodes
                .iter()
                .filter(|n| n.store.subscribe().borrow().first <= block.number())
                .collect();
            // Store this block to one of them.
            if let Some(node) = interested_nodes.choose(rng) {
                node.store.queue_block(ctx, block.clone()).await.unwrap();
            }
            // Wait until all remaining nodes get the new block.
            for node in interested_nodes {
                node.store
                    .wait_until_persisted(ctx, block.number())
                    .await
                    .unwrap();
            }
        }
        Ok(())
    })
    .await
    .unwrap();
}
