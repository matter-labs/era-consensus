use super::*;
use crate::{event::Event, io, preface, rpc, rpc::Rpc as _, run_network, testonly};
use anyhow::Context as _;
use pretty_assertions::assert_eq;
use rand::Rng;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use test_casing::{test_casing, Product};
use tracing::Instrument as _;
use zksync_concurrency::testonly::abort_on_panic;
use zksync_concurrency::{
    ctx::{self, channel},
    oneshot, scope,
    sync::{watch, Mutex},
    time,
};
use zksync_consensus_roles as roles;
use zksync_consensus_roles::validator::{self, BlockNumber, FinalBlock};
use zksync_consensus_utils::pipe;

#[tokio::test]
async fn test_one_connection_per_node() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let mut nodes: Vec<_> = testonly::Instance::new(rng, 5, 2);

    scope::run!(ctx, |ctx,s| async {
        for n in &nodes {
            let (network_pipe, _) = pipe::new();

            s.spawn_bg(run_network(
                ctx,
                n.state.clone(),
                network_pipe
            ));
        }

        tracing::info!("waiting for all connections to be established");
        for node in &mut nodes {
            tracing::info!("node {:?} awaiting connections", node.consensus_config().key.public());
            let want = &node.state.gossip.cfg.static_outbound.keys().cloned().collect();
            let mut subscription = node.state.gossip.outbound.subscribe();
            subscription
                .wait_for(|got| got.current() == want)
                .await
                .unwrap();
        }

        tracing::info!(
            "Impersonate a node, and try to establish additional connection to an already connected peer."
        );
        let my_gossip_config = &nodes[0].state.gossip.cfg;
        let (peer, addr) = my_gossip_config.static_outbound.iter().next().unwrap();
        let mut stream = preface::connect(
            ctx,
            *addr,
            preface::Endpoint::GossipNet,
        )
        .await
        .context("preface::connect")?;

        handshake::outbound(ctx, my_gossip_config, &mut stream, peer)
            .await
            .context("handshake::outbound")?;
        tracing::info!("The connection is expected to be closed automatically by peer.");
        // The multiplexer runner should exit gracefully.
        let _ = rpc::Service::new().run(ctx, stream).await;
        tracing::info!(
            "Exiting the main task. Context will get canceled, all the nodes are expected \
             to terminate gracefully."
        );
        Ok(())
    })
    .await
    .unwrap();
}

fn mk_addr<R: Rng>(rng: &mut R) -> std::net::SocketAddr {
    std::net::SocketAddr::new(std::net::IpAddr::from(rng.gen::<[u8; 16]>()), rng.gen())
}

fn mk_timestamp<R: Rng>(rng: &mut R) -> time::Utc {
    time::UNIX_EPOCH + time::Duration::seconds(rng.gen_range(0..1000000000))
}

fn mk_version<R: Rng>(rng: &mut R) -> u64 {
    rng.gen_range(0..1000)
}

#[derive(Default)]
struct View(im::HashMap<validator::PublicKey, Arc<validator::Signed<validator::NetAddress>>>);

fn mk_netaddr(
    key: &validator::SecretKey,
    addr: std::net::SocketAddr,
    version: u64,
    timestamp: time::Utc,
) -> validator::Signed<validator::NetAddress> {
    key.sign_msg(validator::NetAddress {
        addr,
        version,
        timestamp,
    })
}

fn random_netaddr<R: Rng>(
    rng: &mut R,
    key: &validator::SecretKey,
) -> Arc<validator::Signed<validator::NetAddress>> {
    Arc::new(mk_netaddr(
        key,
        mk_addr(rng),
        mk_version(rng),
        mk_timestamp(rng),
    ))
}

fn update_netaddr<R: Rng>(
    rng: &mut R,
    addr: &validator::NetAddress,
    key: &validator::SecretKey,
    version_diff: i64,
    timestamp_diff: time::Duration,
) -> Arc<validator::Signed<validator::NetAddress>> {
    Arc::new(mk_netaddr(
        key,
        mk_addr(rng),
        (addr.version as i64 + version_diff) as u64,
        addr.timestamp + timestamp_diff,
    ))
}

impl View {
    fn insert(&mut self, entry: Arc<validator::Signed<validator::NetAddress>>) {
        self.0.insert(entry.key.clone(), entry);
    }

    fn get(&mut self, key: &validator::SecretKey) -> Arc<validator::Signed<validator::NetAddress>> {
        self.0.get(&key.public()).unwrap().clone()
    }

    fn as_vec(&self) -> Vec<Arc<validator::Signed<validator::NetAddress>>> {
        self.0.values().cloned().collect()
    }
}

#[tokio::test]
async fn test_validator_addrs() {
    abort_on_panic();
    let rng = &mut ctx::test_root(&ctx::RealClock).rng();

    let keys: Vec<validator::SecretKey> = (0..8).map(|_| rng.gen()).collect();
    let validators = validator::ValidatorSet::new(keys.iter().map(|k| k.public())).unwrap();
    let va = ValidatorAddrsWatch::default();
    let mut sub = va.subscribe();

    // Initial values.
    let mut want = View::default();
    for k in &keys[0..6] {
        want.insert(random_netaddr(rng, k));
    }
    va.update(&validators, &want.as_vec()).await.unwrap();
    assert_eq!(want.0, sub.borrow_and_update().0);

    // Update values.
    let delta = time::Duration::seconds(10);
    // newer version
    let k0v2 = update_netaddr(rng, &want.get(&keys[0]).msg, &keys[0], 1, -delta);
    // same version, newer timestamp
    let k1v2 = update_netaddr(rng, &want.get(&keys[1]).msg, &keys[1], 0, delta);
    // same version, same timestamp
    let k2v2 = update_netaddr(
        rng,
        &want.get(&keys[2]).msg,
        &keys[2],
        0,
        time::Duration::ZERO,
    );
    // same version, older timestamp
    let k3v2 = update_netaddr(rng, &want.get(&keys[3]).msg, &keys[3], 0, -delta);
    // older version
    let k4v2 = update_netaddr(rng, &want.get(&keys[4]).msg, &keys[4], -1, delta);
    // first entry for a key in the config
    let k6v1 = random_netaddr(rng, &keys[6]);
    // entry for a key outside of the config
    let k8 = rng.gen();
    let k8v1 = random_netaddr(rng, &k8);

    want.insert(k0v2.clone());
    want.insert(k1v2.clone());
    want.insert(k6v1.clone());
    let update = [
        k0v2,
        k1v2,
        k2v2,
        k3v2,
        k4v2,
        // no new entry for keys[5]
        k6v1,
        // no entry at all for keys[7]
        k8v1.clone(),
    ];
    va.update(&validators, &update).await.unwrap();
    assert_eq!(want.0, sub.borrow_and_update().0);

    // Invalid signature.
    let mut k0v3 = mk_netaddr(
        &keys[1],
        mk_addr(rng),
        want.get(&keys[0]).msg.version + 1,
        mk_timestamp(rng),
    );
    k0v3.key = keys[0].public();
    assert!(va.update(&validators, &[Arc::new(k0v3)]).await.is_err());
    assert_eq!(want.0, sub.borrow_and_update().0);

    // Duplicate entry in the update.
    assert!(va.update(&validators, &[k8v1.clone(), k8v1]).await.is_err());
    assert_eq!(want.0, sub.borrow_and_update().0);
}

fn to_addr_map(addrs: &ValidatorAddrs) -> HashMap<validator::PublicKey, std::net::SocketAddr> {
    addrs
        .0
        .iter()
        .map(|(k, v)| (k.clone(), v.msg.addr))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_validator_addrs_propagation() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::AffineClock::new(40.));
    let rng = &mut ctx.rng();
    let nodes: Vec<_> = testonly::Instance::new(rng, 10, 1);

    scope::run!(ctx, |ctx, s| async {
        for n in &nodes {
            let (network_pipe, _) = pipe::new();
            s.spawn_bg(run_network(ctx, n.state.clone(), network_pipe));
        }
        let want: HashMap<_, _> = nodes
            .iter()
            .map(|node| {
                (
                    node.consensus_config().key.public(),
                    node.consensus_config().public_addr,
                )
            })
            .collect();
        for (i, node) in nodes.iter().enumerate() {
            tracing::info!("awaiting for node[{i}] to learn validator_addrs");
            node.state
                .gossip
                .validator_addrs
                .subscribe()
                .wait_for(|got| want == to_addr_map(got))
                .await
                .unwrap();
        }
        Ok(())
    })
    .await
    .unwrap();
}

const EXCHANGED_STATE_COUNT: u64 = 5;
const NETWORK_CONNECTIVITY_CASES: [(usize, usize); 5] = [(2, 1), (3, 2), (5, 3), (10, 4), (10, 7)];

/// Tests block syncing with global network synchronization (a next block becoming available
/// to all nodes only after all nodes have received previous `SyncState` updates from peers).
#[test_casing(5, NETWORK_CONNECTIVITY_CASES)]
#[tokio::test(flavor = "multi_thread")]
#[tracing::instrument(level = "trace")]
async fn syncing_blocks(node_count: usize, gossip_peers: usize) {
    abort_on_panic();

    let ctx = &ctx::test_root(&ctx::AffineClock::new(20.0));
    let ctx = &ctx.with_timeout(time::Duration::seconds(200));
    let rng = &mut ctx.rng();
    let mut nodes = testonly::Instance::new(rng, node_count, gossip_peers);
    let network_state = NetworkState::new(rng, &mut nodes);

    scope::run!(ctx, |ctx, s| async {
        for node in nodes {
            let (network_pipe, dispatcher_pipe) = pipe::new();
            s.spawn_bg(run_network(ctx, node.state.clone(), network_pipe));
            s.spawn(network_state.run_mock_dispatcher(ctx, dispatcher_pipe, gossip_peers));
        }
        Ok(())
    })
    .await
    .unwrap();
}

#[derive(Debug)]
struct NetworkStateInner {
    node_count: usize,
    updated_node_count: usize,
    state_sender: watch::Sender<io::SyncState>,
    future_states: Vec<io::SyncState>,
}

/// Network state shared among all nodes in order to coordinate new block generation.
#[derive(Debug)]
struct NetworkState(Mutex<NetworkStateInner>);

impl NetworkState {
    fn new(rng: &mut impl Rng, nodes: &mut [testonly::Instance]) -> Self {
        let node_count = nodes.len();
        let first_state = io::SyncState::gen(rng, BlockNumber(0));
        let future_states = (1..EXCHANGED_STATE_COUNT)
            .map(|i| io::SyncState::gen(rng, BlockNumber(i)))
            .rev() // We `pop()` states, so their ordering should be reversed
            .collect();
        let (state_sender, state_subscriber) = watch::channel(first_state);
        for node in nodes {
            node.disable_gossip_pings();
            node.set_sync_state_subscriber(state_subscriber.clone());
        }

        let inner = NetworkStateInner {
            node_count,
            updated_node_count: 0,
            state_sender,
            future_states,
        };
        Self(Mutex::new(inner))
    }

    async fn update_node_state(&self, received_state: &io::SyncState) {
        let mut inner = self.0.lock().await;
        assert_eq!(*received_state, *inner.state_sender.borrow());

        inner.updated_node_count += 1;
        if inner.updated_node_count == inner.node_count {
            inner.updated_node_count = 0;
            if let Some(next_state) = inner.future_states.pop() {
                inner.state_sender.send_replace(next_state);
            }
        }
    }

    async fn run_mock_dispatcher(
        &self,
        ctx: &ctx::Ctx,
        mut pipe: pipe::DispatcherPipe<io::InputMessage, io::OutputMessage>,
        peer_count: usize,
    ) -> anyhow::Result<()> {
        let mut received_states_by_peer = HashMap::new();
        let mut expected_latest_block_number = BlockNumber(0);
        loop {
            if let io::OutputMessage::SyncBlocks(req) = pipe.recv(ctx).await? {
                match req {
                    io::SyncBlocksRequest::UpdatePeerSyncState {
                        peer,
                        state,
                        response,
                    } => {
                        let last_block_number = state.last_stored_block.message.proposal.number;
                        if last_block_number == expected_latest_block_number {
                            // We might receive outdated states, hence this check
                            received_states_by_peer.insert(peer.clone(), *state.clone());
                        }

                        if received_states_by_peer.len() == peer_count {
                            received_states_by_peer.clear();
                            expected_latest_block_number = expected_latest_block_number.next();
                            if expected_latest_block_number == BlockNumber(EXCHANGED_STATE_COUNT) {
                                // The node has received all generated blocks.
                                return Ok(());
                            }
                            self.update_node_state(&state).await;
                        }
                        response.send(()).ok();
                    }
                    _ => unreachable!(),
                }
            }
        }
    }
}

/// Tests block syncing in an uncoordinated network, in which new blocks arrive at a schedule.
/// In this case, some nodes may skip emitting initial / intermediate updates to peers, so we
/// only assert that all peers for all nodes emit the final update.
#[test_casing(10, Product((
    NETWORK_CONNECTIVITY_CASES,
    [time::Duration::seconds(1), time::Duration::seconds(10)],
)))]
#[tokio::test(flavor = "multi_thread")]
#[tracing::instrument(level = "trace")]
async fn uncoordinated_block_syncing(
    (node_count, gossip_peers): (usize, usize),
    state_generation_interval: time::Duration,
) {
    abort_on_panic();

    let ctx = &ctx::test_root(&ctx::AffineClock::new(20.0));
    let ctx = &ctx.with_timeout(time::Duration::seconds(200));
    let rng = &mut ctx.rng();
    let mut nodes = testonly::Instance::new(rng, node_count, gossip_peers);

    let mut states: Vec<_> = (0..EXCHANGED_STATE_COUNT)
        .map(|i| io::SyncState::gen(rng, BlockNumber(i)))
        .collect();
    let first_state = states.remove(0);
    let (state_sender, state_subscriber) = watch::channel(first_state);

    for node in &mut nodes {
        node.disable_gossip_pings();
        node.set_sync_state_subscriber(state_subscriber.clone());
    }

    scope::run!(ctx, |ctx, s| async {
        s.spawn(async {
            for state in states {
                ctx.sleep(state_generation_interval).await?;
                let last_block_number = state.last_stored_block.message.proposal.number;
                tracing::debug!("Generated `SyncState` with last block number {last_block_number}");
                state_sender.send_replace(state);
            }
            Ok(())
        });

        for node in &nodes {
            let (network_pipe, dispatcher_pipe) = pipe::new();
            s.spawn_bg(run_network(ctx, node.state.clone(), network_pipe));
            let node_key = node.state.gossip.cfg.key.public();
            s.spawn(run_mock_uncoordinated_dispatcher(
                ctx,
                dispatcher_pipe,
                node_key,
                gossip_peers,
            ));
        }
        Ok(())
    })
    .await
    .unwrap();
}

async fn run_mock_uncoordinated_dispatcher(
    ctx: &ctx::Ctx,
    mut pipe: pipe::DispatcherPipe<io::InputMessage, io::OutputMessage>,
    node_key: roles::node::PublicKey,
    peer_count: usize,
) -> anyhow::Result<()> {
    let mut peers_with_final_state = HashSet::new();
    loop {
        if let io::OutputMessage::SyncBlocks(req) = pipe.recv(ctx).await? {
            match req {
                io::SyncBlocksRequest::UpdatePeerSyncState {
                    peer,
                    state,
                    response,
                } => {
                    let last_block_number = state.last_stored_block.message.proposal.number;
                    tracing::debug!(
                        "Node {node_key:?} received update with block number {last_block_number} from {peer:?}"
                    );
                    assert!(last_block_number < BlockNumber(EXCHANGED_STATE_COUNT));
                    if last_block_number == BlockNumber(EXCHANGED_STATE_COUNT - 1) {
                        peers_with_final_state.insert(peer.clone());
                        if peers_with_final_state.len() == peer_count {
                            tracing::debug!("Node {node_key:?} received latest state from peers {peers_with_final_state:?}");
                            return Ok(());
                        }
                    }
                    response.send(()).ok();
                }
                _ => unreachable!(),
            }
        }
    }
}

#[test_casing(5, NETWORK_CONNECTIVITY_CASES)]
#[tokio::test]
async fn getting_blocks_from_peers(node_count: usize, gossip_peers: usize) {
    abort_on_panic();

    let ctx = &ctx::test_root(&ctx::ManualClock::new());
    let rng = &mut ctx.rng();
    let mut nodes = testonly::Instance::new(rng, node_count, gossip_peers);
    for node in &mut nodes {
        node.disable_gossip_pings();
    }
    let node_keys: Vec<_> = nodes
        .iter()
        .map(|node| node.state.gossip.cfg.key.public())
        .collect();

    let block: FinalBlock = rng.gen();
    // All inbound and outbound peers should answer the request.
    let expected_successful_responses = (2 * gossip_peers).min(node_count - 1);

    scope::run!(ctx, |ctx, s| async {
        let node_handles = nodes.iter().map(|node| {
            let (network_pipe, dispatcher_pipe) = pipe::new();
            let (node_stop_sender, node_stop_receiver) = oneshot::channel::<()>();
            s.spawn_bg(async {
                scope::run!(ctx, |ctx, s| async {
                    s.spawn_bg(run_network(ctx, node.state.clone(), network_pipe));
                    s.spawn_bg(async {
                        run_get_block_dispatcher(ctx, dispatcher_pipe.recv, block.clone()).await;
                        Ok(())
                    });
                    node_stop_receiver.recv_or_disconnected(ctx).await.ok();
                    Ok(())
                })
                .await
            });
            (node, node_stop_sender, dispatcher_pipe.send)
        });
        let mut node_handles: Vec<_> = node_handles.collect();

        for (node, _, get_block_sender) in &node_handles {
            node.wait_for_gossip_connections().await;

            let mut successful_peer_responses = 0;
            for peer_key in &node_keys {
                let (response, response_receiver) = oneshot::channel();
                let message = io::SyncBlocksInputMessage::GetBlock {
                    recipient: peer_key.clone(),
                    number: BlockNumber(1),
                    response,
                };
                get_block_sender.send(message.into());
                if response_receiver.recv_or_disconnected(ctx).await?.is_ok() {
                    successful_peer_responses += 1;
                } else {
                    let self_key = node.state.gossip.cfg.key.public();
                    tracing::trace!("Request from {self_key:?} to {peer_key:?} was dropped");
                }
            }
            assert_eq!(successful_peer_responses, expected_successful_responses);
        }

        // Stop the last node by dropping its `node_stop_sender` and wait until it disconnects
        // from all peers.
        node_handles.pop().unwrap();
        let stopped_node_key = node_keys.last().unwrap();
        for (node, _, get_block_sender) in &node_handles {
            let mut inbound_conns = node.state.gossip.inbound.subscribe();
            inbound_conns
                .wait_for(|conns| !conns.current().contains(stopped_node_key))
                .await?;
            let mut outbound_conns = node.state.gossip.outbound.subscribe();
            outbound_conns
                .wait_for(|conns| !conns.current().contains(stopped_node_key))
                .await?;

            // Check that the node cannot access the stopped peer.
            let (response, response_receiver) = oneshot::channel();
            let message = io::SyncBlocksInputMessage::GetBlock {
                recipient: stopped_node_key.clone(),
                number: BlockNumber(1),
                response,
            };
            get_block_sender.send(message.into());
            assert!(response_receiver.recv_or_disconnected(ctx).await?.is_err());
        }

        Ok(())
    })
    .await
    .unwrap();
}

async fn run_get_block_dispatcher(
    ctx: &ctx::Ctx,
    mut receiver: channel::UnboundedReceiver<io::OutputMessage>,
    block: FinalBlock,
) {
    while let Ok(message) = receiver.recv(ctx).await {
        match message {
            io::OutputMessage::SyncBlocks(io::SyncBlocksRequest::GetBlock {
                block_number,
                response,
            }) => {
                assert_eq!(block_number, BlockNumber(1));
                response.send(Ok(block.clone())).ok();
            }
            other => panic!("received unexpected message: {other:?}"),
        }
    }
}

/// When validator node is restarted, it should immediately override
/// the AccountData that is present in the network from the previous run.
#[tokio::test]
async fn validator_node_restart() {
    abort_on_panic();
    let clock = &ctx::ManualClock::new();
    let ctx = &ctx::test_root(clock);
    let rng = &mut ctx.rng();

    let zero = time::Duration::ZERO;
    let sec = time::Duration::seconds(1);

    scope::run!(ctx, |ctx, s| async {
        let mut cfgs = testonly::Instance::new_configs(rng, 2, 1);
        let mut node1 = testonly::Instance::from_cfg(cfgs[1].clone());
        let (pipe, _) = pipe::new();
        s.spawn_bg(
            run_network(ctx, node1.state.clone(), pipe).instrument(tracing::info_span!("node1")),
        );

        // We restart the node0 after shifting the UTC clock back and forth.
        // node0 is expected to learn what was is the currently broadcasted
        // ValidatorAddr and broadcast another one with the corrected value.
        let mut utc_times = HashSet::new();
        let start = ctx.now_utc();
        for clock_shift in [zero, sec, -2 * sec, 4 * sec, 10 * sec, -30 * sec] {
            // Set the new addr to broadcast.
            let mutated_config = cfgs[0].consensus.as_mut().unwrap();
            let key0 = mutated_config.key.public();
            let addr0 = mk_addr(rng);
            mutated_config.public_addr = addr0;
            // Shift the UTC clock.
            let now = start + clock_shift;
            assert!(
                utc_times.insert(now),
                "UTC time has to be unique for the broadcast to be guaranteed to succeed"
            );
            clock.set_utc(now);
            tracing::info!("now = {now:?}");

            scope::run!(ctx, |ctx, s| async {
                let node0 = testonly::Instance::from_cfg(cfgs[0].clone());
                let (pipe, _) = pipe::new();
                s.spawn_bg(
                    run_network(ctx, node0.state.clone(), pipe)
                        .instrument(tracing::info_span!("node0")),
                );
                s.spawn_bg(async {
                    // Progress time whenever node1 receives an update.
                    // TODO(gprusak): alternatively we could entirely disable time progress
                    // by setting refresh time to 0 in tests.
                    while let Ok(ev) = node1.events.recv(ctx).await {
                        if let Event::ValidatorAddrsUpdated = ev {
                            clock.advance(rpc::sync_validator_addrs::Rpc::RATE.refresh);
                        }
                    }
                    Ok(())
                });
                node1
                    .state
                    .gossip
                    .validator_addrs
                    .subscribe()
                    .wait_for(|got| {
                        let Some(got) = got.get(&key0) else {
                            return false;
                        };
                        tracing::info!("got.addr = {}", got.msg.addr);
                        got.msg.addr == addr0
                    })
                    .await
                    .unwrap();
                Ok(())
            })
            .await?;
        }
        Ok(())
    })
    .await
    .unwrap();
}

/// Test that SyncValidatorAddrs RPC batches updates
/// and is rate limited. Test is constructing a gossip
/// network with star topology. All nodes are expected
/// to receive all updates in 2 rounds of communication.
#[tokio::test]
async fn rate_limiting() {
    abort_on_panic();
    let clock = &ctx::ManualClock::new();
    let ctx = &ctx::test_root(clock);
    let rng = &mut ctx.rng();

    // construct star topology.
    let n = 10;
    let mut cfgs = testonly::Instance::new_configs(rng, n, 0);
    let want: HashMap<_, _> = cfgs
        .iter()
        .map(|cfg| {
            let consensus_cfg = cfg.consensus.as_ref().unwrap();
            (consensus_cfg.key.public(), consensus_cfg.public_addr)
        })
        .collect();
    for i in 1..n {
        let key = cfgs[i].gossip.key.public().clone();
        let public_addr = cfgs[i].consensus.as_ref().unwrap().public_addr;
        cfgs[0].gossip.static_outbound.insert(key, public_addr);
    }
    let mut nodes: Vec<_> = cfgs.into_iter().map(testonly::Instance::from_cfg).collect();

    scope::run!(ctx, |ctx, s| async {
        // Spawn the satellite nodes and wait until they register
        // their own address.
        for (i, node) in nodes.iter().enumerate().skip(1) {
            let (pipe, _) = pipe::new();
            s.spawn_bg(
                run_network(ctx, node.state.clone(), pipe)
                    .instrument(tracing::info_span!("node", i)),
            );
            node.state
                .gossip
                .validator_addrs
                .subscribe()
                .wait_for(|got| got.get(&node.consensus_config().key.public()).is_some())
                .await
                .unwrap();
        }
        // Spawn the center node.
        let (pipe, _) = pipe::new();
        s.spawn_bg(
            run_network(ctx, nodes[0].state.clone(), pipe)
                .instrument(tracing::info_span!("node[0]")),
        );
        // Await for the center to receive all validator addrs.
        nodes[0]
            .state
            .gossip
            .validator_addrs
            .subscribe()
            .wait_for(|got| want == to_addr_map(got))
            .await
            .unwrap();
        // Advance time and wait for all other nodes to receive validator addrs.
        clock.advance(rpc::sync_validator_addrs::Rpc::RATE.refresh);
        for node in &nodes[1..n] {
            node.state
                .gossip
                .validator_addrs
                .subscribe()
                .wait_for(|got| want == to_addr_map(got))
                .await
                .unwrap();
        }
        Ok(())
    })
    .await
    .unwrap();

    // Check that the satellite nodes received either 1 or 2 updates.
    for n in &mut nodes[1..] {
        let mut count = 0;
        while let Some(ev) = n.events.try_recv() {
            if let Event::ValidatorAddrsUpdated = ev {
                count += 1;
            }
        }
        assert!((1..=2).contains(&count));
    }
}
