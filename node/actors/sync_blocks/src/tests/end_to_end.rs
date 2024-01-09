//! End-to-end tests that launch a network of nodes and the `SyncBlocks` actor for each node.
use super::*;
use anyhow::Context as _;
use async_trait::async_trait;
use rand::seq::SliceRandom;
use std::fmt;
use test_casing::test_casing;
use tracing::{instrument, Instrument};
use zksync_concurrency::{ctx, scope, sync, testonly::abort_on_panic};
use zksync_consensus_network as network;
use zksync_consensus_network::{io::SyncState, testonly::Instance as NetworkInstance};
use zksync_consensus_roles::node;
use zksync_consensus_utils::no_copy::NoCopy;

type NetworkDispatcherPipe =
    pipe::DispatcherPipe<network::io::InputMessage, network::io::OutputMessage>;

#[derive(Debug)]
struct Node {
    store: Arc<BlockStore>,
    test_validators: Arc<TestValidators>,
    switch_on_sender: Option<oneshot::Sender<()>>,
    _switch_off_sender: oneshot::Sender<()>,
}

impl Node {
    async fn new_network(
        ctx: &ctx::Ctx,
        node_count: usize,
        gossip_peers: usize,
    ) -> (Vec<Node>, Vec<NodeRunner>) {
        let rng = &mut ctx.rng();
        let test_validators = Arc::new(TestValidators::new(rng, 4, 20));
        let mut nodes = vec![];
        let mut runners = vec![];
        for net in NetworkInstance::new(rng, node_count, gossip_peers) {
            let (n, r) = Node::new(ctx, net, test_validators.clone()).await;
            nodes.push(n);
            runners.push(r);
        }
        (nodes, runners)
    }

    async fn new(
        ctx: &ctx::Ctx,
        mut network: NetworkInstance,
        test_validators: Arc<TestValidators>,
    ) -> (Self, NodeRunner) {
        let (store, store_runner) = make_store(ctx, test_validators.final_blocks[0].clone()).await;
        let (switch_on_sender, switch_on_receiver) = oneshot::channel();
        let (switch_off_sender, switch_off_receiver) = oneshot::channel();

        network.disable_gossip_pings();

        let runner = NodeRunner {
            network,
            store: store.clone(),
            store_runner,
            test_validators: test_validators.clone(),
            switch_on_receiver,
            switch_off_receiver,
        };
        let this = Self {
            store,
            test_validators,
            switch_on_sender: Some(switch_on_sender),
            _switch_off_sender: switch_off_sender,
        };
        (this, runner)
    }

    fn switch_on(&mut self) {
        self.switch_on_sender.take();
    }

    async fn put_block(&self, ctx: &ctx::Ctx, block_number: BlockNumber) {
        tracing::trace!(%block_number, "Storing new block");
        let block = &self.test_validators.final_blocks[block_number.0 as usize];
        self.store.queue_block(ctx, block.clone()).await.unwrap();
    }
}

#[must_use]
struct NodeRunner {
    network: NetworkInstance,
    store: Arc<BlockStore>,
    store_runner: BlockStoreRunner,
    test_validators: Arc<TestValidators>,
    switch_on_receiver: oneshot::Receiver<()>,
    switch_off_receiver: oneshot::Receiver<()>,
}

impl fmt::Debug for NodeRunner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeRunner")
            .field("key", &self.key())
            .finish()
    }
}

fn to_sync_state(state: BlockStoreState) -> SyncState {
    SyncState {
        first_stored_block: state.first,
        last_stored_block: state.last,
    }
}

impl NodeRunner {
    fn key(&self) -> node::PublicKey {
        self.network.gossip_config().key.public()
    }

    async fn run(mut self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        let key = self.key();
        let (sync_blocks_actor_pipe, sync_blocks_dispatcher_pipe) = pipe::new();
        let (network_actor_pipe, network_dispatcher_pipe) = pipe::new();
        let mut store_state = self.store.subscribe();
        let sync_state = sync::watch::channel(to_sync_state(store_state.borrow().clone())).0;
        self.network
            .set_sync_state_subscriber(sync_state.subscribe());

        let sync_blocks_config = self.test_validators.test_config();
        scope::run!(ctx, |ctx, s| async {
            s.spawn_bg(self.store_runner.run(ctx));
            s.spawn_bg(async {
                while let Ok(state) = sync::changed(ctx, &mut store_state).await {
                    sync_state.send_replace(to_sync_state(state.clone()));
                }
                Ok(())
            });
            s.spawn_bg(async {
                network::run_network(ctx, self.network.state().clone(), network_actor_pipe)
                    .instrument(tracing::trace_span!("network", ?key))
                    .await
                    .with_context(|| format!("network for {key:?}"))
            });
            self.network.wait_for_gossip_connections().await;
            tracing::trace!("Node connected to peers");

            self.switch_on_receiver
                .recv_or_disconnected(ctx)
                .await?
                .ok();
            s.spawn_bg(async {
                Self::run_executor(ctx, sync_blocks_dispatcher_pipe, network_dispatcher_pipe)
                    .instrument(tracing::trace_span!("mock_executor", ?key))
                    .await
                    .with_context(|| format!("executor for {key:?}"))
            });
            s.spawn_bg(sync_blocks_config.run(ctx, sync_blocks_actor_pipe, self.store.clone()));
            tracing::info!("Node is fully started");

            self.switch_off_receiver
                .recv_or_disconnected(ctx)
                .await
                .ok();
            // ^ Unlike with `switch_on_receiver`, the context may get canceled before the receiver
            // is dropped, so we swallow both cancellation and disconnect errors here.
            tracing::info!("Node stopped");
            Ok(())
        })
        .await
    }

    async fn run_executor(
        ctx: &ctx::Ctx,
        mut sync_blocks_dispatcher_pipe: pipe::DispatcherPipe<InputMessage, OutputMessage>,
        mut network_dispatcher_pipe: NetworkDispatcherPipe,
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
    const CLOCK_SPEEDUP: u32 = 25;

    abort_on_panic();

    let ctx = &ctx::test_root(&ctx::AffineClock::new(CLOCK_SPEEDUP as f64))
        .with_timeout(TEST_TIMEOUT * CLOCK_SPEEDUP);
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

        // Check initial node states.
        for node_handle in &mut node_handles {
            node_handle.switch_on();
            let state = node_handle.store.subscribe().borrow().clone();
            assert_eq!(state.first.header().number, BlockNumber(0));
            assert_eq!(state.last.header().number, BlockNumber(0));
        }

        for block_number in (1..5).map(BlockNumber) {
            let sending_node = node_handles.choose(rng).unwrap();
            sending_node.put_block(ctx, block_number).await;

            // Wait until all nodes get this block.
            for node_handle in &mut node_handles {
                wait_for_stored_block(ctx, &node_handle.store, block_number).await?;
            }
            tracing::trace!("All nodes received block #{block_number}");
        }

        let sending_node = node_handles.choose(rng).unwrap();
        scope::run!(ctx, |ctx, s| async {
            // Add a batch of blocks.
            for block_number in (5..10).rev().map(BlockNumber) {
                let block_number = NoCopy::from(block_number);
                s.spawn_bg(async {
                    sending_node.put_block(ctx, block_number.into_inner()).await;
                    Ok(())
                });
            }

            // Wait until nodes get all new blocks.
            for node_handle in &node_handles {
                wait_for_stored_block(ctx, &node_handle.store, BlockNumber(9)).await?;
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

    async fn test(self, ctx: &ctx::Ctx, mut node_handles: Vec<Node>) -> anyhow::Result<()> {
        let rng = &mut ctx.rng();

        for node_handle in &mut node_handles {
            node_handle.switch_on();
        }

        let mut block_number = BlockNumber(1);
        while !node_handles.is_empty() {
            tracing::info!("{} nodes left", node_handles.len());

            let sending_node = node_handles.choose(rng).unwrap();
            sending_node.put_block(ctx, block_number).await;
            tracing::info!("block {block_number} inserted");

            // Wait until all remaining nodes get the new block.
            for node_handle in &node_handles {
                wait_for_stored_block(ctx, &node_handle.store, block_number).await?;
            }
            tracing::trace!("All nodes received block #{block_number}");
            block_number = block_number.next();

            // Switch off a random node by dropping its handle.
            // We start switching off only after the first round, to make sure all nodes are fully
            // started.
            let node_index_to_remove = rng.gen_range(0..node_handles.len());
            node_handles.swap_remove(node_index_to_remove);
        }
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
        let mut block_number = BlockNumber(1);
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
                wait_for_stored_block(ctx, &node_handle.store, block_number).await?;
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
