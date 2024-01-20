use super::*;
use crate::{event::Event, io, preface, rpc, rpc::Rpc as _, testonly};
use anyhow::Context as _;
use pretty_assertions::assert_eq;
use rand::Rng;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use test_casing::{test_casing, Product};
use tracing::Instrument as _;
use zksync_concurrency::{
    ctx,
    oneshot, scope,
    testonly::abort_on_panic,
    time,
};
use zksync_consensus_roles::validator::{self, BlockNumber, FinalBlock};
use zksync_consensus_utils::no_copy::NoCopy;

#[tokio::test]
async fn test_one_connection_per_node() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let setup = validator::testonly::GenesisSetup::new(rng, 5);
    let cfgs = testonly::new_configs(rng, &setup, 2);

    scope::run!(ctx, |ctx,s| async {
        let (store,runner) = testonly::new_store(ctx,&setup.blocks[0]).await;
        s.spawn_bg(runner.run(ctx));
        let mut nodes : Vec<_> = cfgs.iter().enumerate().map(|(i,cfg)| {
            let (node,runner) = testonly::Instance::new(cfg.clone(), store.clone());
            s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node", i)));
            node
        }).collect();

        tracing::info!("waiting for all connections to be established");
        for node in &mut nodes {
            node.wait_for_gossip_connections().await; 
        }

        tracing::info!(
            "Impersonate a node, and try to establish additional connection to an already connected peer."
        );
        let (peer, addr) = cfgs[0].gossip.static_outbound.iter().next().unwrap();
        let mut stream = preface::connect(
            ctx,
            *addr,
            preface::Endpoint::GossipNet,
        )
        .await
        .context("preface::connect")?;

        handshake::outbound(ctx, &cfgs[0].gossip, &mut stream, peer)
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
    let setup = validator::testonly::GenesisSetup::new(rng, 10);
    let cfgs = testonly::new_configs(rng, &setup, 1);

    scope::run!(ctx, |ctx, s| async {
        let (store,runner) = testonly::new_store(ctx,&setup.blocks[0]).await;
        s.spawn_bg(runner.run(ctx));
        let nodes : Vec<_> = cfgs.iter().enumerate().map(|(i,cfg)| {
            let (node,runner) = testonly::Instance::new(cfg.clone(), store.clone());
            s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node", i)));
            node
        }).collect();
        let want: HashMap<_, _> = cfgs
            .iter()
            .map(|cfg| {
                let cfg = cfg.consensus.as_ref().unwrap();
                (cfg.key.public(), cfg.public_addr)
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

const EXCHANGED_STATE_COUNT: usize = 5;
const NETWORK_CONNECTIVITY_CASES: [(usize, usize); 5] = [(2, 1), (3, 2), (5, 3), (10, 4), (10, 7)];

/// Tests block syncing with global network synchronization (a next block becoming available
/// to all nodes only after all nodes have received previous `SyncState` updates from peers).
#[test_casing(5, NETWORK_CONNECTIVITY_CASES)]
#[tokio::test(flavor = "multi_thread")]
#[tracing::instrument(level = "trace")]
async fn syncing_blocks(node_count: usize, gossip_peers: usize) {
    abort_on_panic();

    let ctx = &ctx::test_root(&ctx::AffineClock::new(20.0));
    let rng = &mut ctx.rng();
    let mut setup = validator::testonly::GenesisSetup::empty(rng, node_count);
    setup.push_blocks(rng,EXCHANGED_STATE_COUNT);
    scope::run!(ctx, |ctx, s| async { 
        s.spawn_bg(async {
            match ctx.sleep(time::Duration::seconds(200)).await {
                Ok(()) => panic!("TIMEOUT"),
                Err(_) => Ok(())
            }
        });
        for (i,cfg) in testonly::new_configs(rng, &setup, gossip_peers).into_iter().enumerate() {
            s.spawn(async {
                let (store,runner) = testonly::new_store(ctx,&setup.blocks[0]).await;
                s.spawn_bg(runner.run(ctx));
                let (mut node,runner) = testonly::Instance::new(cfg, store);
                s.spawn_bg(runner.run(ctx));
                run_mock_dispatcher(ctx, &mut node, gossip_peers, &setup.blocks).await
                    .context("run_mock_dispatcher()")?;
                s.spawn_bg(async {
                    let mut node = node;
                    node.auto_ack(ctx).await;
                    Ok(())
                });
                tracing::info!("received all updates");
                Ok(())
            }.instrument(tracing::info_span!("node",i)));
        }
        Ok(())
    })
    .await
    .unwrap();
}

async fn run_mock_dispatcher(
    ctx: &ctx::Ctx,
    node: &mut testonly::Instance,
    peer_count: usize,
    blocks: &[FinalBlock],
) -> anyhow::Result<()> {
    for (i,block) in blocks.iter().enumerate() {
        tracing::info!("block {i}");
        let mut updated_peers = HashSet::new();
        node.state.gossip.block_store.queue_block(ctx,block.clone()).await.context("queue_block()")?;
        while updated_peers.len() < peer_count {
            let io::OutputMessage::SyncBlocks(io::SyncBlocksRequest::UpdatePeerSyncState {
                peer,
                state,
                response,
            }) = node.pipe.recv(ctx).await.context("pipe.recv()")? else { continue };
            if state.last == block.justification {
                // We might receive outdated states, hence this check
                updated_peers.insert(peer);
            }
            response.send(()).ok();
        }
    }
    Ok(())
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
    let rng = &mut ctx.rng();
    let mut setup = validator::testonly::GenesisSetup::empty(rng, node_count);
    setup.push_blocks(rng,EXCHANGED_STATE_COUNT);
    scope::run!(ctx, |ctx, s| async {
        for (i,cfg) in testonly::new_configs(rng, &setup, gossip_peers).into_iter().enumerate() {
            let i = NoCopy::from(i);
            s.spawn(async {
                let i = i;
                let (store,runner) = testonly::new_store(ctx,&setup.blocks[0]).await;
                s.spawn_bg(runner.run(ctx));
                let (mut node,runner) = testonly::Instance::new(cfg,store.clone());
                s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node",i=*i)));
                s.spawn_bg(async {
                    let store = store;
                    for block in &setup.blocks[1..] {
                        ctx.sleep(state_generation_interval).await?;
                        store.queue_block(ctx,block.clone()).await.unwrap();
                    }
                    Ok(())
                });
                run_mock_uncoordinated_dispatcher(
                    ctx,
                    &mut node,
                    gossip_peers,
                    setup.blocks.last().unwrap().header().number,
                ).await.context("run_mock_uncoordinated_dispatcher()")  
            });
        }
        Ok(())
    })
    .await
    .unwrap();
}

async fn run_mock_uncoordinated_dispatcher(
    ctx: &ctx::Ctx,
    node: &mut testonly::Instance,
    peer_count: usize,
    want_last: BlockNumber,
) -> anyhow::Result<()> {
    let mut peers_with_final_state = HashSet::new();
    while peers_with_final_state.len() < peer_count {
        let io::OutputMessage::SyncBlocks(io::SyncBlocksRequest::UpdatePeerSyncState {
            peer,
            state,
            response,
        }) = node.pipe.recv(ctx).await? else { continue };
        let got_last = state.last.header().number;
        assert!(got_last <= want_last);
        if got_last == want_last {
            peers_with_final_state.insert(peer.clone());
        }
        response.send(()).ok();
    }
    Ok(())
}

#[test_casing(5, NETWORK_CONNECTIVITY_CASES)]
#[tokio::test]
async fn getting_blocks_from_peers(node_count: usize, gossip_peers: usize) {
    abort_on_panic();

    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let setup = validator::testonly::GenesisSetup::new(rng, node_count);
    let cfgs = testonly::new_configs(rng, &setup, gossip_peers);

    // All inbound and outbound peers should answer the request.
    let expected_successful_responses = (2 * gossip_peers).min(node_count - 1);

    let (store,runner) = testonly::new_store(ctx,&setup.blocks[0]).await;
    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(runner.run(ctx));
        let mut nodes : Vec<_> = cfgs.into_iter().enumerate().map(|(i,cfg)| {
            let (node,runner) = testonly::Instance::new(cfg, store.clone());
            s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node",i)));
            node
        }).collect();
        
        for node in &nodes {
            node.wait_for_gossip_connections().await;
            tracing::info!("establish connections");
            let mut successful_peer_responses = 0;
            for peer in &nodes {
                let (response, response_receiver) = oneshot::channel();
                node.pipe.send(io::SyncBlocksInputMessage::GetBlock {
                    recipient: peer.state.gossip.cfg.key.public(),
                    number: setup.blocks[0].header().number,
                    response,
                }.into());
                tracing::info!("wait for response");
                if let Ok(block) = response_receiver.recv(ctx).await? {
                    assert_eq!(block,setup.blocks[0]);
                    successful_peer_responses += 1;
                }
            }
            assert_eq!(successful_peer_responses, expected_successful_responses);
        }

        tracing::info!("stop the last node");
        let last = nodes.pop().unwrap();
        last.terminate.send();
        
        let stopped_node_key = last.state.gossip.cfg.key.public();
        for node in &nodes {
            tracing::info!("wait for disconnection");
            node.wait_for_gossip_disconnect(ctx,&stopped_node_key).await.unwrap();

            tracing::info!("wait for disconnection");
            // Check that the node cannot access the stopped peer.
            let (response, response_receiver) = oneshot::channel();
            node.pipe.send(io::SyncBlocksInputMessage::GetBlock {
                recipient: stopped_node_key.clone(),
                number: BlockNumber(1),
                response,
            }.into());
            assert!(response_receiver.recv(ctx).await?.is_err());
        }

        Ok(())
    })
    .await
    .unwrap();
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
        let setup = validator::testonly::GenesisSetup::new(rng, 2);
        let (store,runner) = testonly::new_store(ctx,&setup.blocks[0]).await;
        s.spawn_bg(runner.run(ctx));
        let mut cfgs = testonly::new_configs(rng, &setup, 1);
        let (mut node1,runner) = testonly::Instance::new(cfgs[1].clone(),store.clone());
        s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node1")));

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
                let (_,runner) = testonly::Instance::new(cfgs[0].clone(),store.clone());
                s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node0")));
                s.spawn_bg(async {
                    // Progress time whenever node1 receives an update.
                    // TODO(gprusak): alternatively we could entirely disable time progress
                    // by setting refresh time to 0 in tests.
                    while let Ok(Event::ValidatorAddrsUpdated) = node1.events.recv(ctx).await {
                        clock.advance(rpc::push_validator_addrs::Rpc::RATE.refresh);
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
    let setup = validator::testonly::GenesisSetup::new(rng, n);
    let mut cfgs = testonly::new_configs(rng, &setup, 0);
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
    let mut nodes = vec![];
    scope::run!(ctx, |ctx, s| async {
        let (store,runner) = testonly::new_store(ctx,&setup.blocks[0]).await;
        s.spawn_bg(runner.run(ctx));
        // Spawn the satellite nodes and wait until they register
        // their own address.
        for (i,cfg) in cfgs[1..].iter().enumerate() {
            let (node,runner) = testonly::Instance::new(cfg.clone(),store.clone());
            s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node", i)));
            node.state
                .gossip
                .validator_addrs
                .subscribe()
                .wait_for(|got| got.get(&node.consensus_config().key.public()).is_some())
                .await
                .unwrap();
            nodes.push(node);
        }
        
        // Spawn the center node.
        let (center,runner) = testonly::Instance::new(cfgs[0].clone(),store.clone());
        s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node[0]")));
        // Await for the center to receive all validator addrs.
        center
            .state
            .gossip
            .validator_addrs
            .subscribe()
            .wait_for(|got| want == to_addr_map(got))
            .await
            .unwrap();
        // Advance time and wait for all other nodes to receive validator addrs.
        clock.advance(rpc::push_validator_addrs::Rpc::RATE.refresh);
        for node in &nodes {
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
    for n in &mut nodes {
        let mut count = 0;
        while let Some(Event::ValidatorAddrsUpdated) = n.events.try_recv() {
            count += 1;
        }
        assert!((1..=2).contains(&count));
    }
}
