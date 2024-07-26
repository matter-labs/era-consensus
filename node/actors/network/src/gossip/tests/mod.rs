use super::ValidatorAddrs;
use crate::{
    gossip::{
        batch_votes::{BatchVotes, BatchVotesWatch},
        handshake,
        validator_addrs::ValidatorAddrsWatch,
    },
    metrics, preface, rpc, testonly,
};
use anyhow::Context as _;
use assert_matches::assert_matches;
use pretty_assertions::assert_eq;
use rand::Rng;
use std::{
    collections::{HashMap, HashSet},
    sync::{atomic::Ordering, Arc},
};
use tracing::Instrument as _;
use zksync_concurrency::{
    ctx, net, scope, sync,
    testonly::{abort_on_panic, set_timeout},
    time,
};
use zksync_consensus_roles::{attester, validator};
use zksync_consensus_storage::{testonly::TestMemoryStorage, PersistentBatchStore};

mod fetch_batches;
mod fetch_blocks;
mod syncing;

#[tokio::test]
async fn test_one_connection_per_node() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let setup = validator::testonly::Setup::new(rng, 5);
    let cfgs = testonly::new_configs(rng, &setup, 2);

    scope::run!(ctx, |ctx,s| async {
        let store = TestMemoryStorage::new(ctx, &setup.genesis).await;
        s.spawn_bg(store.runner.run(ctx));
        let mut nodes : Vec<_> = cfgs.iter().enumerate().map(|(i,cfg)| {
            let (node,runner) = testonly::Instance::new(cfg.clone(), store.blocks.clone(), store.batches.clone());
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
            addr.resolve(ctx).await.unwrap().unwrap()[0],
            preface::Endpoint::GossipNet,
        )
        .await
        .context("preface::connect")?;

        handshake::outbound(ctx, &cfgs[0].gossip, setup.genesis.hash(), &mut stream, peer)
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

#[derive(Default)]
struct Signatures(im::HashMap<attester::PublicKey, Arc<attester::Signed<attester::Batch>>>);

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

fn mk_batch<R: Rng>(
    rng: &mut R,
    key: &attester::SecretKey,
    number: attester::BatchNumber,
    genesis: attester::GenesisHash,
) -> attester::Signed<attester::Batch> {
    key.sign_msg(attester::Batch {
        number,
        hash: rng.gen(),
        genesis,
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

fn random_batch_vote<R: Rng>(
    rng: &mut R,
    key: &attester::SecretKey,
    genesis: attester::GenesisHash,
) -> Arc<attester::Signed<attester::Batch>> {
    let batch = attester::Batch {
        number: attester::BatchNumber(rng.gen_range(0..1000)),
        hash: rng.gen(),
        genesis,
    };
    Arc::new(key.sign_msg(batch.to_owned()))
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

fn update_signature<R: Rng>(
    rng: &mut R,
    batch: &attester::Batch,
    key: &attester::SecretKey,
    batch_number_diff: i64,
) -> Arc<attester::Signed<attester::Batch>> {
    let batch = attester::Batch {
        number: attester::BatchNumber((batch.number.0 as i64 + batch_number_diff) as u64),
        hash: rng.gen(),
        genesis: batch.genesis,
    };
    Arc::new(key.sign_msg(batch.to_owned()))
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

impl Signatures {
    fn insert(&mut self, entry: Arc<attester::Signed<attester::Batch>>) {
        self.0.insert(entry.key.clone(), entry);
    }

    fn get(&mut self, key: &attester::SecretKey) -> Arc<attester::Signed<attester::Batch>> {
        self.0.get(&key.public()).unwrap().clone()
    }

    fn as_vec(&self) -> Vec<Arc<attester::Signed<attester::Batch>>> {
        self.0.values().cloned().collect()
    }
}

#[tokio::test]
async fn test_validator_addrs() {
    abort_on_panic();
    let rng = &mut ctx::test_root(&ctx::RealClock).rng();

    let keys: Vec<validator::SecretKey> = (0..8).map(|_| rng.gen()).collect();
    let validators = validator::Committee::new(keys.iter().map(|k| validator::WeightedValidator {
        key: k.public(),
        weight: 1250,
    }))
    .unwrap();
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
    let setup = validator::testonly::Setup::new(rng, 10);
    let cfgs = testonly::new_configs(rng, &setup, 1);

    scope::run!(ctx, |ctx, s| async {
        let store = TestMemoryStorage::new(ctx, &setup.genesis).await;
        s.spawn_bg(store.runner.run(ctx));
        let nodes: Vec<_> = cfgs
            .iter()
            .enumerate()
            .map(|(i, cfg)| {
                let (node, runner) = testonly::Instance::new(
                    cfg.clone(),
                    store.blocks.clone(),
                    store.batches.clone(),
                );
                s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node", i)));
                node
            })
            .collect();
        let want: HashMap<_, _> = cfgs
            .iter()
            .map(|cfg| {
                (
                    cfg.validator_key.as_ref().unwrap().public(),
                    *cfg.server_addr,
                )
            })
            .collect();
        for (i, node) in nodes.iter().enumerate() {
            tracing::info!("awaiting for node[{i}] to learn validator_addrs");
            let sub = &mut node.net.gossip.validator_addrs.subscribe();
            sync::wait_for(ctx, sub, |got| want == to_addr_map(got)).await?;
        }
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_genesis_mismatch() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let setup = validator::testonly::Setup::new(rng, 2);
    let cfgs = testonly::new_configs(rng, &setup, 1);

    scope::run!(ctx, |ctx, s| async {
        let mut listener = cfgs[1].server_addr.bind().context("server_addr.bind()")?;

        tracing::info!("Start one node, we will simulate the other one.");
        let store = TestMemoryStorage::new(ctx, &setup.genesis).await;
        s.spawn_bg(store.runner.run(ctx));
        let (_node, runner) =
            testonly::Instance::new(cfgs[0].clone(), store.blocks, store.batches.clone());
        s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node")));

        tracing::info!("Accept a connection with mismatching genesis.");
        let stream = metrics::MeteredStream::accept(ctx, &mut listener)
            .await?
            .context("accept()")?;
        let (mut stream, endpoint) = preface::accept(ctx, stream)
            .await
            .context("preface::accept()")?;
        assert_eq!(endpoint, preface::Endpoint::GossipNet);
        tracing::info!("Expect the handshake to fail");
        let res = handshake::inbound(ctx, &cfgs[1].gossip, rng.gen(), &mut stream).await;
        assert_matches!(res, Err(handshake::Error::GenesisMismatch));

        tracing::info!("Try to connect to a node with a mismatching genesis.");
        let mut stream = preface::connect(ctx, *cfgs[0].server_addr, preface::Endpoint::GossipNet)
            .await
            .context("preface::connect")?;
        let res = handshake::outbound(
            ctx,
            &cfgs[1].gossip,
            rng.gen(),
            &mut stream,
            &cfgs[0].gossip.key.public(),
        )
        .await;
        tracing::info!(
            "Expect the peer to verify the mismatching Genesis and close the connection."
        );
        assert_matches!(res, Err(handshake::Error::Stream(_)));
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
    let _guard = set_timeout(time::Duration::seconds(5));

    let clock = ctx::ManualClock::new();
    let ctx = &ctx::test_root(&clock);
    let rng = &mut ctx.rng();

    let zero = time::Duration::ZERO;
    let sec = time::Duration::seconds(1);

    let setup = validator::testonly::Setup::new(rng, 2);
    let mut cfgs = testonly::new_configs(rng, &setup, 1);
    // Set the rpc refresh time to 0, so that any updates are immediately propagated.
    for cfg in &mut cfgs {
        cfg.rpc.push_validator_addrs_rate.refresh = time::Duration::ZERO;
    }
    let store = TestMemoryStorage::new(ctx, &setup.genesis).await;
    let (node1, node1_runner) =
        testonly::Instance::new(cfgs[1].clone(), store.blocks.clone(), store.batches.clone());
    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(store.runner.run(ctx));
        s.spawn_bg(
            node1_runner
                .run(ctx)
                .instrument(tracing::info_span!("node1")),
        );

        // We restart the node0 after shifting the UTC clock back and forth.
        // node0 is expected to learn what was is the currently broadcasted
        // ValidatorAddr and broadcast another one with the corrected value.
        let mut utc_times = HashSet::new();
        let start = ctx.now_utc();
        for clock_shift in [zero, sec, -2 * sec, 4 * sec, 10 * sec, -30 * sec] {
            // Set the new addr to broadcast.
            cfgs[0].server_addr = net::tcp::testonly::reserve_listener();
            cfgs[0].public_addr = (*cfgs[0].server_addr).into();
            // Shift the UTC clock.
            let now = start + clock_shift;
            assert!(
                utc_times.insert(now),
                "UTC time has to be unique for the broadcast to be guaranteed to succeed"
            );
            clock.set_utc(now);
            tracing::info!("now = {now:?}");

            // _node0 contains pipe, which has to exist to prevent the connection from dying
            // early.
            let (_node0, runner) = testonly::Instance::new(
                cfgs[0].clone(),
                store.blocks.clone(),
                store.batches.clone(),
            );
            scope::run!(ctx, |ctx, s| async {
                s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node0")));
                tracing::info!("wait for the update to arrive to node1");
                let sub = &mut node1.net.gossip.validator_addrs.subscribe();
                let want = Some(*cfgs[0].server_addr);
                sync::wait_for(ctx, sub, |got| {
                    got.get(&setup.validator_keys[0].public())
                        .map(|x| x.msg.addr)
                        == want
                })
                .await?;
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
    let setup = validator::testonly::Setup::new(rng, n);
    let mut cfgs = testonly::new_configs(rng, &setup, 0);
    let want: HashMap<_, _> = cfgs
        .iter()
        .map(|cfg| {
            (
                cfg.validator_key.as_ref().unwrap().public(),
                *cfg.server_addr,
            )
        })
        .collect();
    for i in 1..n {
        let key = cfgs[i].gossip.key.public().clone();
        let public_addr = cfgs[i].public_addr.clone();
        cfgs[0].gossip.static_outbound.insert(key, public_addr);
    }
    let mut nodes = vec![];
    scope::run!(ctx, |ctx, s| async {
        let store = TestMemoryStorage::new(ctx, &setup.genesis).await;
        s.spawn_bg(store.runner.run(ctx));
        // Spawn the satellite nodes and wait until they register
        // their own address.
        for (i, cfg) in cfgs[1..].iter().enumerate() {
            let (node, runner) =
                testonly::Instance::new(cfg.clone(), store.blocks.clone(), store.batches.clone());
            s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node", i)));
            let sub = &mut node.net.gossip.validator_addrs.subscribe();
            sync::wait_for(ctx, sub, |got| {
                got.get(&node.cfg().validator_key.as_ref().unwrap().public())
                    .is_some()
            })
            .await
            .unwrap();
            nodes.push(node);
        }

        // Spawn the center node.
        let (center, runner) =
            testonly::Instance::new(cfgs[0].clone(), store.blocks.clone(), store.batches.clone());
        s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node[0]")));
        // Await for the center to receive all validator addrs.
        let sub = &mut center.net.gossip.validator_addrs.subscribe();
        sync::wait_for(ctx, sub, |got| want == to_addr_map(got)).await?;
        // Advance time and wait for all other nodes to receive validator addrs.
        clock.advance(center.cfg().rpc.push_validator_addrs_rate.refresh);
        for node in &nodes {
            let sub = &mut node.net.gossip.validator_addrs.subscribe();
            sync::wait_for(ctx, sub, |got| want == to_addr_map(got)).await?;
        }
        Ok(())
    })
    .await
    .unwrap();

    // Check that the satellite nodes received either 1 or 2 updates.
    for n in &mut nodes {
        let got = n
            .net
            .gossip
            .push_validator_addrs_calls
            .load(Ordering::SeqCst);
        assert!((1..=2).contains(&got), "got {got} want 1 or 2");
    }
}

#[tokio::test]
async fn test_batch_votes() {
    abort_on_panic();
    let rng = &mut ctx::test_root(&ctx::RealClock).rng();

    let keys: Vec<attester::SecretKey> = (0..8).map(|_| rng.gen()).collect();
    let attesters = attester::Committee::new(keys.iter().map(|k| attester::WeightedAttester {
        key: k.public(),
        weight: 1250,
    }))
    .unwrap();
    let votes = BatchVotesWatch::default();
    let mut sub = votes.subscribe();

    let genesis = rng.gen::<attester::GenesisHash>();

    // Initial values.
    let mut want = Signatures::default();
    for k in &keys[0..6] {
        want.insert(random_batch_vote(rng, k, genesis));
    }
    votes
        .update(&attesters, &genesis, &want.as_vec())
        .await
        .unwrap();
    assert_eq!(want.0, sub.borrow_and_update().votes);

    // newer batch number, should be updated
    let k0v2 = update_signature(rng, &want.get(&keys[0]).msg, &keys[0], 1);
    // same batch number, should be ignored
    let k1v2 = update_signature(rng, &want.get(&keys[1]).msg, &keys[1], 0);
    // older batch number, should be ignored
    let k4v2 = update_signature(rng, &want.get(&keys[4]).msg, &keys[4], -1);
    // first entry for a key in the config, should be inserted
    let k6v1 = random_batch_vote(rng, &keys[6], genesis);
    // entry for a key outside of the config, should be ignored
    let k8 = rng.gen();
    let k8v1 = random_batch_vote(rng, &k8, genesis);

    // Update the ones we expect to succeed
    want.insert(k0v2.clone());
    want.insert(k6v1.clone());

    // Send all of them to the votes
    let update = [
        k0v2,
        k1v2,
        k4v2,
        // no new entry for keys[5]
        k6v1,
        // no entry at all for keys[7]
        k8v1.clone(),
    ];
    votes.update(&attesters, &genesis, &update).await.unwrap();
    assert_eq!(want.0, sub.borrow_and_update().votes);

    // Invalid signature, should be rejected.
    let mut k0v3 = mk_batch(
        rng,
        &keys[1],
        attester::BatchNumber(want.get(&keys[0]).msg.number.0 + 1),
        genesis,
    );
    k0v3.key = keys[0].public();
    assert!(votes
        .update(&attesters, &genesis, &[Arc::new(k0v3)])
        .await
        .is_err());

    // Invalid genesis, should be rejected.
    let other_genesis = rng.gen();
    let k1v3 = mk_batch(
        rng,
        &keys[1],
        attester::BatchNumber(want.get(&keys[1]).msg.number.0 + 1),
        other_genesis,
    );
    assert!(votes
        .update(&attesters, &genesis, &[Arc::new(k1v3)])
        .await
        .is_err());

    assert_eq!(want.0, sub.borrow_and_update().votes);

    // Duplicate entry in the update, should be rejected.
    assert!(votes
        .update(&attesters, &genesis, &[k8v1.clone(), k8v1])
        .await
        .is_err());
    assert_eq!(want.0, sub.borrow_and_update().votes);
}

#[test]
fn test_batch_votes_quorum() {
    abort_on_panic();
    let rng = &mut ctx::test_root(&ctx::RealClock).rng();

    for _ in 0..10 {
        let size = rng.gen_range(1..20);
        let keys: Vec<attester::SecretKey> = (0..size).map(|_| rng.gen()).collect();
        let attesters = attester::Committee::new(keys.iter().map(|k| attester::WeightedAttester {
            key: k.public(),
            weight: rng.gen_range(1..=100),
        }))
        .unwrap();

        let batch0 = rng.gen::<attester::Batch>();
        let batch1 = attester::Batch {
            number: batch0.number.next(),
            hash: rng.gen(),
            genesis: batch0.genesis,
        };
        let genesis = batch0.genesis;
        let mut batches = [(batch0, 0u64), (batch1, 0u64)];

        let mut votes = BatchVotes::default();
        for sk in &keys {
            // We need 4/5+1 for quorum, so let's say ~80% vote on the second batch.
            let b = usize::from(rng.gen_range(0..100) < 80);
            let batch = &batches[b].0;
            let vote = sk.sign_msg(batch.clone());
            votes
                .update(&attesters, &genesis, &[Arc::new(vote)])
                .unwrap();
            batches[b].1 += attesters.weight(&sk.public()).unwrap();

            // Check that as soon as we have quorum it's found.
            if batches[b].1 >= attesters.threshold() {
                let qs = votes.find_quorums(&attesters, &genesis, |_| false);
                assert!(!qs.is_empty(), "should find quorum");
                assert!(qs[0].message == *batch);
                assert!(qs[0].signatures.keys().count() > 0);
            }
        }

        if let Some(quorum) = batches
            .iter()
            .find(|b| b.1 >= attesters.threshold())
            .map(|(b, _)| b)
        {
            // Check that a quorum can be skipped
            assert!(votes
                .find_quorums(&attesters, &genesis, |b| b == quorum.number)
                .is_empty());
        } else {
            // Check that if there was no quoroum then we don't find any.
            assert!(votes
                .find_quorums(&attesters, &genesis, |_| false)
                .is_empty());
        }

        // Check that the minimum batch number prunes data.
        let last_batch = batches[1].0.number;

        votes.set_min_batch_number(last_batch);
        assert!(votes.votes.values().all(|v| v.msg.number >= last_batch));
        assert!(votes.support.keys().all(|n| *n >= last_batch));

        votes.set_min_batch_number(last_batch.next());
        assert!(votes.votes.is_empty());
        assert!(votes.support.is_empty());
    }
}

//
#[tokio::test(flavor = "multi_thread")]
async fn test_batch_votes_propagation() {
    let _guard = set_timeout(time::Duration::seconds(10));
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::AffineClock::new(10.));
    let rng = &mut ctx.rng();

    let mut setup = validator::testonly::Setup::new(rng, 10);

    let cfgs = testonly::new_configs(rng, &setup, 1);

    scope::run!(ctx, |ctx, s| async {
        // All nodes share the same in-memory store
        let store = TestMemoryStorage::new(ctx, &setup.genesis).await;
        s.spawn_bg(store.runner.run(ctx));

        // Push the first batch into the store so we have something to vote on.
        setup.push_blocks(rng, 2);
        setup.push_batch(rng);

        let batch = setup.batches[0].clone();

        store
            .batches
            .queue_batch(ctx, batch.clone(), setup.genesis.clone())
            .await
            .unwrap();

        let batch = attester::Batch {
            number: batch.number,
            hash: rng.gen(),
            genesis: setup.genesis.hash(),
        };

        // Start all nodes.
        let nodes: Vec<testonly::Instance> = cfgs
            .iter()
            .enumerate()
            .map(|(i, cfg)| {
                let (node, runner) = testonly::Instance::new(
                    cfg.clone(),
                    store.blocks.clone(),
                    store.batches.clone(),
                );
                s.spawn_bg(runner.run(ctx).instrument(tracing::info_span!("node", i)));
                node
            })
            .collect();

        // Cast votes on each node. It doesn't matter who signs where in this test,
        // we just happen to know that we'll have as many nodes as attesters.
        let attesters = setup.genesis.attesters.as_ref().unwrap();
        for (node, key) in nodes.iter().zip(setup.attester_keys.iter()) {
            node.net
                .batch_vote_publisher()
                .publish(attesters, &setup.genesis.hash(), key, batch.clone())
                .await
                .unwrap();
        }

        // Wait until one of the nodes collects a quorum over gossip and stores.
        loop {
            if let Some(qc) = store.im_batches.get_batch_qc(ctx, batch.number).await? {
                assert_eq!(qc.message, batch);
                return Ok(());
            }
            ctx.sleep(time::Duration::milliseconds(100)).await?;
        }
    })
    .await
    .unwrap();
}
