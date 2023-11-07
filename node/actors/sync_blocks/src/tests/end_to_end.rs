//! End-to-end tests that launch a network of nodes and the `SyncBlocks` actor for each node.
use super::*;
use anyhow::Context as _;
use async_trait::async_trait;
use rand::seq::SliceRandom;
use std::fmt;
use test_casing::test_casing;
use tracing::Instrument;
use zksync_concurrency::{ctx::channel, testonly::abort_on_panic};
use zksync_consensus_network as network;
use zksync_consensus_network::testonly::Instance as NetworkInstance;
use zksync_consensus_roles::node;
use zksync_consensus_storage::InMemoryStorage;

type NetworkDispatcherPipe =
    pipe::DispatcherPipe<network::io::InputMessage, network::io::OutputMessage>;

#[derive(Debug)]
struct NodeHandle {
    create_block_sender: channel::UnboundedSender<BlockNumber>,
    sync_state_subscriber: watch::Receiver<SyncState>,
    switch_on_sender: Option<oneshot::Sender<()>>,
    _switch_off_sender: oneshot::Sender<()>,
}

impl NodeHandle {
    fn switch_on(&mut self) {
        self.switch_on_sender.take();
    }
}

#[derive(Debug)]
struct InitialNodeHandle {
    create_block_sender: channel::UnboundedSender<BlockNumber>,
    sync_state_subscriber_receiver: oneshot::Receiver<watch::Receiver<SyncState>>,
    switch_on_sender: oneshot::Sender<()>,
    _switch_off_sender: oneshot::Sender<()>,
}

impl InitialNodeHandle {
    async fn wait(self, ctx: &ctx::Ctx) -> anyhow::Result<NodeHandle> {
        let sync_state_subscriber = self
            .sync_state_subscriber_receiver
            .recv_or_disconnected(ctx)
            .await??;
        Ok(NodeHandle {
            create_block_sender: self.create_block_sender,
            sync_state_subscriber,
            switch_on_sender: Some(self.switch_on_sender),
            _switch_off_sender: self._switch_off_sender,
        })
    }
}

struct Node {
    network: NetworkInstance,
    /// Receiver to command a node to push a block with the specified number to its storage.
    create_block_receiver: channel::UnboundedReceiver<BlockNumber>,
    sync_state_subscriber_sender: oneshot::Sender<watch::Receiver<SyncState>>,
    switch_on_receiver: oneshot::Receiver<()>,
    switch_off_receiver: oneshot::Receiver<()>,
}

impl fmt::Debug for Node {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Node")
            .field("key", &self.key())
            .finish()
    }
}

impl Node {
    fn new(mut network: NetworkInstance) -> (Self, InitialNodeHandle) {
        let (create_block_sender, create_block_receiver) = channel::unbounded();
        let (sync_state_subscriber_sender, sync_state_subscriber_receiver) = oneshot::channel();
        let (switch_on_sender, switch_on_receiver) = oneshot::channel();
        let (switch_off_sender, switch_off_receiver) = oneshot::channel();

        network.disable_gossip_pings();

        let this = Self {
            network,
            create_block_receiver,
            sync_state_subscriber_sender,
            switch_on_receiver,
            switch_off_receiver,
        };
        let handle = InitialNodeHandle {
            create_block_sender,
            sync_state_subscriber_receiver,
            switch_on_sender,
            _switch_off_sender: switch_off_sender,
        };
        (this, handle)
    }

    fn key(&self) -> node::PublicKey {
        self.network.gossip_config().key.public()
    }

    #[instrument(level = "trace", skip(ctx, test_validators), err)]
    async fn run(mut self, ctx: &ctx::Ctx, test_validators: &TestValidators) -> anyhow::Result<()> {
        let key = self.key();
        let (sync_blocks_actor_pipe, sync_blocks_dispatcher_pipe) = pipe::new();
        let (network_actor_pipe, network_dispatcher_pipe) = pipe::new();
        let storage = InMemoryStorage::new(test_validators.final_blocks[0].clone());
        let storage = Arc::new(storage);

        let sync_blocks_config = test_validators.test_config();
        let sync_blocks = SyncBlocks::new(
            ctx,
            sync_blocks_actor_pipe,
            storage.clone(),
            sync_blocks_config,
        )
        .await
        .expect("Failed initializing `sync_blocks` actor");

        let sync_states_subscriber = sync_blocks.subscribe_to_state_updates();
        self.network
            .set_sync_state_subscriber(sync_states_subscriber.clone());

        scope::run!(ctx, |ctx, s| async {
            s.spawn_bg(async {
                while let Ok(block_number) = self.create_block_receiver.recv(ctx).await {
                    tracing::trace!(?key, %block_number, "Storing new block");
                    let block = &test_validators.final_blocks[block_number.0 as usize];
                    storage.put_block(ctx, block).await.unwrap();
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
            self.sync_state_subscriber_sender
                .send(sync_states_subscriber)
                .ok();

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
            s.spawn_bg(sync_blocks.run(ctx));
            tracing::trace!("Node is fully started");

            self.switch_off_receiver
                .recv_or_disconnected(ctx)
                .await
                .ok();
            // ^ Unlike with `switch_on_receiver`, the context may get canceled before the receiver
            // is dropped, so we swallow both cancellation and disconnect errors here.
            tracing::trace!("Node stopped");
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
            let network_task = async {
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
            };
            s.spawn(network_task.instrument(tracing::Span::current()));

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

#[derive(Debug)]
struct GossipNetwork<H = NodeHandle> {
    test_validators: TestValidators,
    node_handles: Vec<H>,
}

impl GossipNetwork<InitialNodeHandle> {
    fn new(rng: &mut impl Rng, node_count: usize, gossip_peers: usize) -> (Self, Vec<Node>) {
        let test_validators = TestValidators::new(4, 20, rng);
        let nodes = NetworkInstance::new(rng, node_count, gossip_peers);
        let (nodes, node_handles) = nodes.into_iter().map(Node::new).unzip();
        let this = Self {
            test_validators,
            node_handles,
        };
        (this, nodes)
    }
}

#[async_trait]
trait GossipNetworkTest: fmt::Debug + Send {
    /// Returns the number of nodes in the gossip network and number of peers for each node.
    fn network_params(&self) -> (usize, usize);

    async fn test(self, ctx: &ctx::Ctx, network: GossipNetwork) -> anyhow::Result<()>;
}

#[instrument(level = "trace")]
async fn test_sync_blocks<T: GossipNetworkTest>(test: T) {
    const CLOCK_SPEEDUP: u32 = 25;

    abort_on_panic();

    let ctx = &ctx::test_root(&ctx::AffineClock::new(CLOCK_SPEEDUP as f64))
        .with_timeout(TEST_TIMEOUT * CLOCK_SPEEDUP);
    let (node_count, gossip_peers) = test.network_params();
    let (network, nodes) = GossipNetwork::new(&mut ctx.rng(), node_count, gossip_peers);
    scope::run!(ctx, |ctx, s| async {
        for node in nodes {
            let test_validators = network.test_validators.clone();
            s.spawn_bg(async {
                let test_validators = test_validators;
                let key = node.key();
                node.run(ctx, &test_validators).await?;
                tracing::trace!(?key, "Node task completed");
                Ok(())
            });
        }

        let mut node_handles = Vec::with_capacity(network.node_handles.len());
        for node_handle in network.node_handles {
            node_handles.push(node_handle.wait(ctx).await?);
        }
        tracing::trace!("Finished preparations for test");

        let network = GossipNetwork {
            test_validators: network.test_validators,
            node_handles,
        };
        test.test(ctx, network).await
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

    async fn test(self, ctx: &ctx::Ctx, network: GossipNetwork) -> anyhow::Result<()> {
        let GossipNetwork {
            mut node_handles, ..
        } = network;
        let rng = &mut ctx.rng();

        // Check initial node states.
        for node_handle in &mut node_handles {
            node_handle.switch_on();
            let block_numbers = node_handle.sync_state_subscriber.borrow().numbers();
            assert_eq!(block_numbers.first_stored_block, BlockNumber(0));
            assert_eq!(block_numbers.last_stored_block, BlockNumber(0));
            assert_eq!(block_numbers.last_stored_block, BlockNumber(0));
        }

        for block_number in 1..5 {
            let block_number = BlockNumber(block_number);
            let sending_node = node_handles.choose(rng).unwrap();
            sending_node.create_block_sender.send(block_number);

            // Wait until all nodes get this block.
            for node_handle in &mut node_handles {
                sync::wait_for(ctx, &mut node_handle.sync_state_subscriber, |state| {
                    state.numbers().last_contiguous_stored_block == block_number
                })
                .await?;
            }
            tracing::trace!("All nodes received block #{block_number}");
        }

        // Add blocks in the opposite order, so that other nodes will start downloading all blocks
        // in batch.
        let sending_node = node_handles.choose(rng).unwrap();
        for block_number in (5..10).rev() {
            let block_number = BlockNumber(block_number);
            sending_node.create_block_sender.send(block_number);
        }

        // Wait until nodes get all new blocks.
        for node_handle in &mut node_handles {
            sync::wait_for(ctx, &mut node_handle.sync_state_subscriber, |state| {
                state.numbers().last_contiguous_stored_block == BlockNumber(9)
            })
            .await?;
        }
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

    async fn test(self, ctx: &ctx::Ctx, network: GossipNetwork) -> anyhow::Result<()> {
        let GossipNetwork {
            mut node_handles, ..
        } = network;
        let rng = &mut ctx.rng();

        for node_handle in &mut node_handles {
            node_handle.switch_on();
        }

        let mut block_number = BlockNumber(1);
        while node_handles.len() > 1 {
            // Switch off a random node by dropping its handle.
            let node_index_to_remove = rng.gen_range(0..node_handles.len());
            node_handles.swap_remove(node_index_to_remove);

            let sending_node = node_handles.choose(rng).unwrap();
            sending_node.create_block_sender.send(block_number);

            // Wait until all remaining nodes get the new block.
            for node_handle in &mut node_handles {
                sync::wait_for(ctx, &mut node_handle.sync_state_subscriber, |state| {
                    state.numbers().last_contiguous_stored_block == block_number
                })
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

    async fn test(self, ctx: &ctx::Ctx, network: GossipNetwork) -> anyhow::Result<()> {
        let GossipNetwork {
            mut node_handles, ..
        } = network;
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
            sending_node.create_block_sender.send(block_number);

            // Wait until all switched on nodes get the new block.
            for node_handle in &mut switched_on_nodes {
                sync::wait_for(ctx, &mut node_handle.sync_state_subscriber, |state| {
                    state.numbers().last_contiguous_stored_block == block_number
                })
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
