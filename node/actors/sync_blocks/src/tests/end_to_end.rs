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
use zksync_consensus_storage::testonly::new_store;

type NetworkDispatcherPipe =
    pipe::DispatcherPipe<network::io::InputMessage, network::io::OutputMessage>;

#[derive(Debug)]
struct Node {
    store: Arc<BlockStore>,
    setup: Arc<GenesisSetup>,
    switch_on_sender: Option<oneshot::Sender<()>>,
    terminate: channel::Sender<()>,
}

impl Node {
    async fn new_network(
        ctx: &ctx::Ctx,
        node_count: usize,
        gossip_peers: usize,
    ) -> (Vec<Node>, Vec<NodeRunner>) {
        let rng = &mut ctx.rng();
        // NOTE: originally there were only 4 consensus nodes.
        let mut setup = validator::testonly::GenesisSetup::new(rng, node_count);
        setup.push_blocks(rng, 20);
        let setup = Arc::new(setup);
        let mut nodes = vec![];
        let mut runners = vec![];
        for net in network::testonly::new_configs(rng, &setup, gossip_peers) {
            let (n, r) = Node::new(ctx, net, setup.clone()).await;
            nodes.push(n);
            runners.push(r);
        }
        (nodes, runners)
    }

    async fn new(
        ctx: &ctx::Ctx,
        network: network::Config,
        setup: Arc<GenesisSetup>,
    ) -> (Self, NodeRunner) {
        let (store, store_runner) = new_store(ctx, &setup.genesis).await;
        let (switch_on_sender, switch_on_receiver) = oneshot::channel();
        let (terminate_send, terminate_recv) = channel::bounded(1);

        let runner = NodeRunner {
            network,
            store: store.clone(),
            store_runner,
            setup: setup.clone(),
            switch_on_receiver,
            terminate: terminate_recv,
        };
        let this = Self {
            store,
            setup,
            switch_on_sender: Some(switch_on_sender),
            terminate: terminate_send,
        };
        (this, runner)
    }

    fn switch_on(&mut self) {
        self.switch_on_sender.take();
    }

    async fn terminate(&self, ctx: &ctx::Ctx) -> ctx::OrCanceled<()> {
        let _ = self.terminate.try_send(());
        self.terminate.closed(ctx).await
    }

    async fn put_block(&self, ctx: &ctx::Ctx, block_number: BlockNumber) {
        tracing::trace!(%block_number, "Storing new block");
        let block = &self.setup.blocks[block_number.0 as usize];
        self.store.queue_block(ctx, block.clone()).await.unwrap();
    }
}

#[must_use]
struct NodeRunner {
    network: network::Config,
    store: Arc<BlockStore>,
    store_runner: BlockStoreRunner,
    setup: Arc<GenesisSetup>,
    switch_on_receiver: oneshot::Receiver<()>,
    terminate: channel::Receiver<()>,
}

impl NodeRunner {
    async fn run(mut self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        tracing::info!("NodeRunner::run()");
        let key = self.network.gossip.key.public();
        let (sync_blocks_actor_pipe, sync_blocks_dispatcher_pipe) = pipe::new();
        let (mut network, network_runner) =
            network::testonly::Instance::new(ctx, self.network.clone(), self.store.clone());
        let sync_blocks_config = Config::new(self.setup.genesis.clone());
        let res = scope::run!(ctx, |ctx, s| async {
            s.spawn_bg(self.store_runner.run(ctx));
            s.spawn_bg(network_runner.run(ctx));
            network.wait_for_gossip_connections().await;
            tracing::info!("Node connected to peers");

            self.switch_on_receiver
                .recv_or_disconnected(ctx)
                .await?
                .ok();
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
    async fn test(self, ctx: &ctx::Ctx, network: Vec<Node>) -> anyhow::Result<()>;
}

#[instrument(level = "trace")]
async fn test_sync_blocks<T: GossipNetworkTest>(test: T) {
    abort_on_panic();
    let _guard = set_timeout(TEST_TIMEOUT);
    let ctx = &ctx::test_root(&ctx::AffineClock::new(25.));
    let (node_count, gossip_peers) = test.network_params();
    let (nodes, runners) = Node::new_network(ctx, node_count, gossip_peers).await;
    scope::run!(ctx, |ctx, s| async {
        for (i, runner) in runners.into_iter().enumerate() {
            s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node", i)));
        }
        test.test(ctx, nodes).await
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

    async fn test(self, ctx: &ctx::Ctx, mut node_handles: Vec<Node>) -> anyhow::Result<()> {
        let rng = &mut ctx.rng();

        tracing::info!("Check initial node states");
        for node_handle in &mut node_handles {
            node_handle.switch_on();
            let state = node_handle.store.subscribe().borrow().clone();
            assert_eq!(state.first, BlockNumber(0));
            assert_eq!(state.last, None);
        }

        for block_number in (0..5).map(BlockNumber) {
            let sending_node = node_handles.choose(rng).unwrap();
            sending_node.put_block(ctx, block_number).await;

            tracing::info!("Wait until all nodes get block #{block_number}");
            for node_handle in &mut node_handles {
                node_handle
                    .store
                    .wait_until_persisted(ctx, block_number)
                    .await?;
                tracing::info!("OK");
            }
        }

        let sending_node = node_handles.choose(rng).unwrap();
        scope::run!(ctx, |ctx, s| async {
            // Add a batch of blocks.
            for block_number in (5..10).rev().map(BlockNumber) {
                let block_number = ctx::NoCopy(block_number);
                s.spawn_bg(async {
                    sending_node.put_block(ctx, block_number.into()).await;
                    Ok(())
                });
            }

            // Wait until nodes get all new blocks.
            for node_handle in &node_handles {
                node_handle
                    .store
                    .wait_until_persisted(ctx, BlockNumber(9))
                    .await?;
            }
            Ok(())
        })
        .await
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

    async fn test(self, ctx: &ctx::Ctx, mut nodes: Vec<Node>) -> anyhow::Result<()> {
        let rng = &mut ctx.rng();

        for node in &mut nodes {
            node.switch_on();
        }
        nodes.shuffle(rng);

        let mut block_number = BlockNumber(0);
        while !nodes.is_empty() {
            tracing::info!("{} nodes left", nodes.len());

            nodes
                .choose(rng)
                .unwrap()
                .put_block(ctx, block_number)
                .await;
            tracing::info!("block {block_number} inserted");

            // Wait until all remaining nodes get the new block.
            for node in &nodes {
                node.store.wait_until_persisted(ctx, block_number).await?;
            }
            tracing::info!("All nodes received block #{block_number}");
            block_number = block_number.next();

            // Switch off a random node by dropping its handle.
            // We start switching off only after the first round, to make sure all nodes are fully
            // started.
            nodes.pop().unwrap().terminate(ctx).await.unwrap();
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

    async fn test(self, ctx: &ctx::Ctx, mut node_handles: Vec<Node>) -> anyhow::Result<()> {
        let rng = &mut ctx.rng();

        let mut switched_on_nodes = Vec::with_capacity(self.node_count);
        let mut block_number = BlockNumber(0);
        while switched_on_nodes.len() < self.node_count {
            // Switch on a random node.
            let node_index_to_switch_on = rng.gen_range(0..node_handles.len());
            let mut node_handle = node_handles.swap_remove(node_index_to_switch_on);
            node_handle.switch_on();
            switched_on_nodes.push(node_handle);

            let sending_node = switched_on_nodes.choose(rng).unwrap();
            sending_node.put_block(ctx, block_number).await;

            // Wait until all switched on nodes get the new block.
            for node_handle in &mut switched_on_nodes {
                node_handle
                    .store
                    .wait_until_persisted(ctx, block_number)
                    .await?;
            }
            tracing::trace!("All nodes received block #{block_number}");
            block_number = block_number.next();
        }
        Ok(())
    }
}

#[test_casing(5, 3..=7)]
#[tokio::test(flavor = "multi_thread")]
async fn switching_on_nodes(node_count: usize) {
    test_sync_blocks(SwitchingOnNodes { node_count }).await;
}
