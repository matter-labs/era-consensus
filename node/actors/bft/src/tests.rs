use std::collections::HashMap;

use crate::testonly::{
    twins::{Cluster, HasKey, ScenarioGenerator, Twin},
    ut_harness::UTHarness,
    Behavior, Network, PortRouter, PortSplitSchedule, Test, NUM_PHASES,
};
use zksync_concurrency::{ctx, scope, time};
use zksync_consensus_network::testonly::new_configs_for_validators;
use zksync_consensus_roles::validator::{
    self,
    testonly::{Setup, SetupSpec},
    LeaderSelectionMode, PublicKey, SecretKey, ViewNumber,
};

async fn run_test(behavior: Behavior, network: Network) {
    let _guard = zksync_concurrency::testonly::set_timeout(time::Duration::seconds(30));
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);

    const NODES: usize = 11;
    let mut nodes = vec![(behavior, 1u64); NODES];
    // validator::threshold(NODES) will calculate required nodes to validate a message
    // given each node weight is 1
    let honest_nodes_amount = validator::threshold(NODES as u64) as usize;
    for n in &mut nodes[0..honest_nodes_amount] {
        n.0 = Behavior::Honest;
    }
    Test {
        network,
        nodes,
        blocks_to_finalize: 10,
    }
    .run(ctx)
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn honest_mock_network() {
    run_test(Behavior::Honest, Network::Mock).await
}

#[tokio::test(flavor = "multi_thread")]
async fn honest_real_network() {
    run_test(Behavior::Honest, Network::Real).await
}

#[tokio::test(flavor = "multi_thread")]
async fn offline_mock_network() {
    run_test(Behavior::Offline, Network::Mock).await
}

#[tokio::test(flavor = "multi_thread")]
async fn offline_real_network() {
    run_test(Behavior::Offline, Network::Real).await
}

#[tokio::test(flavor = "multi_thread")]
async fn random_mock_network() {
    run_test(Behavior::Random, Network::Mock).await
}

#[tokio::test(flavor = "multi_thread")]
async fn random_real_network() {
    run_test(Behavior::Random, Network::Real).await
}

#[tokio::test(flavor = "multi_thread")]
async fn byzantine_mock_network() {
    run_test(Behavior::Byzantine, Network::Mock).await
}

#[tokio::test(flavor = "multi_thread")]
async fn byzantine_real_network() {
    run_test(Behavior::Byzantine, Network::Real).await
}

/// Testing liveness after the network becomes idle with leader having no cached prepare messages for the current view.
#[tokio::test]
async fn timeout_leader_no_prepares() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new_many(ctx).await;
        s.spawn_bg(runner.run(ctx));
        util.new_replica_prepare();
        util.produce_block_after_timeout(ctx).await;
        Ok(())
    })
    .await
    .unwrap();
}

/// Testing liveness after the network becomes idle with leader having some cached prepare messages for the current view.
#[tokio::test]
async fn timeout_leader_some_prepares() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new_many(ctx).await;
        s.spawn_bg(runner.run(ctx));
        let replica_prepare = util.new_replica_prepare();
        assert!(util
            .process_replica_prepare(ctx, util.sign(replica_prepare))
            .await
            .unwrap()
            .is_none());
        util.produce_block_after_timeout(ctx).await;
        Ok(())
    })
    .await
    .unwrap();
}

/// Testing liveness after the network becomes idle with leader in commit phase.
#[tokio::test]
async fn timeout_leader_in_commit() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new_many(ctx).await;
        s.spawn_bg(runner.run(ctx));

        util.new_leader_prepare(ctx).await;
        // Leader is in `Phase::Commit`, but should still accept prepares from newer views.
        assert_eq!(util.leader.phase, validator::Phase::Commit);
        util.produce_block_after_timeout(ctx).await;
        Ok(())
    })
    .await
    .unwrap();
}

/// Testing liveness after the network becomes idle with replica in commit phase.
#[tokio::test]
async fn timeout_replica_in_commit() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new_many(ctx).await;
        s.spawn_bg(runner.run(ctx));

        util.new_replica_commit(ctx).await;
        // Leader is in `Phase::Commit`, but should still accept prepares from newer views.
        assert_eq!(util.leader.phase, validator::Phase::Commit);
        util.produce_block_after_timeout(ctx).await;
        Ok(())
    })
    .await
    .unwrap();
}

/// Testing liveness after the network becomes idle with leader having some cached commit messages for the current view.
#[tokio::test]
async fn timeout_leader_some_commits() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new_many(ctx).await;
        s.spawn_bg(runner.run(ctx));

        let replica_commit = util.new_replica_commit(ctx).await;
        assert!(util
            .process_replica_commit(ctx, util.sign(replica_commit))
            .await
            .unwrap()
            .is_none());
        // Leader is in `Phase::Commit`, but should still accept prepares from newer views.
        assert_eq!(util.leader_phase(), validator::Phase::Commit);
        util.produce_block_after_timeout(ctx).await;
        Ok(())
    })
    .await
    .unwrap();
}

/// Testing liveness after the network becomes idle with leader in a consecutive prepare phase.
#[tokio::test]
async fn timeout_leader_in_consecutive_prepare() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async {
        let (mut util, runner) = UTHarness::new_many(ctx).await;
        s.spawn_bg(runner.run(ctx));

        util.new_leader_commit(ctx).await;
        util.produce_block_after_timeout(ctx).await;
        Ok(())
    })
    .await
    .unwrap();
}

/// Not being able to propose a block shouldn't cause a deadlock.
#[tokio::test]
async fn non_proposing_leader() {
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::AffineClock::new(5.));
    Test {
        network: Network::Real,
        nodes: vec![(Behavior::Honest, 1), (Behavior::HonestNotProposing, 1)],
        blocks_to_finalize: 10,
    }
    .run(ctx)
    .await
    .unwrap()
}

/// Run Twins scenarios without actual twins, and with so few nodes that all
/// of them are required for a quorum, which means (currently) there won't be
/// any partitions.
///
/// This should be a simple sanity check that the network works and consensus
/// is achieved under the most favourable conditions.
#[tokio::test(flavor = "multi_thread")]
async fn twins_network_wo_twins_wo_partitions() {
    // n<6 implies f=0 and q=n
    run_twins(5, 0, 10).await.unwrap();
}

/// Run Twins scenarios without actual twins, but enough replicas that partitions
/// can play a role, isolating certain nodes (potentially the leader) in some
/// rounds.
///
/// This should be a sanity check that without Byzantine behaviour the consensus
/// is resilient to temporary network partitions.
#[tokio::test(flavor = "multi_thread")]
async fn twins_network_wo_twins_w_partitions() {
    // n=6 implies f=1 and q=5; 6 is the minimum where partitions are possible.
    run_twins(6, 0, 5).await.unwrap();
}

/// Run Twins scenarios with random number of nodes and 1 twin.
#[tokio::test(flavor = "multi_thread")]
async fn twins_network_w1_twins_w_partitions() {
    // n>=6 implies f>=1 and q=n-f
    for num_replicas in 6..=10 {
        // let num_honest = validator::threshold(num_replicas as u64) as usize;
        // let max_faulty = num_replicas - num_honest;
        // let num_twins = rng.gen_range(1..=max_faulty);
        run_twins(num_replicas, 1, 10).await.unwrap();
    }
}

/// Run Twins scenarios with higher number of nodes and 2 twins.
#[tokio::test(flavor = "multi_thread")]
async fn twins_network_w2_twins_w_partitions() {
    // n>=11 implies f>=2 and q=n-f
    run_twins(11, 2, 10).await.unwrap();
}

/// Run Twins scenario with more twins than tolerable and expect it to fail.
#[tokio::test(flavor = "multi_thread")]
#[should_panic]
async fn twins_network_to_fail() {
    // With n=5 f=0, so 1 twin means more faulty nodes than expected.
    run_twins(5, 1, 100).await.unwrap();
}

/// Create network configuration for a given number of replicas and twins and run [Test].
async fn run_twins(
    num_replicas: usize,
    num_twins: usize,
    num_scenarios: usize,
) -> anyhow::Result<()> {
    let num_honest = validator::threshold(num_replicas as u64) as usize;
    let max_faulty = num_replicas - num_honest;

    // If we pass more twins than tolerable faulty replicas then it should fail with an assertion error,
    // but if we abort the process on panic then the #[should_panic] attribute doesn't work with `cargo nextest`.
    if num_twins <= max_faulty {
        zksync_concurrency::testonly::abort_on_panic();
    }
    zksync_concurrency::testonly::init_tracing();

    // Use a single timeout for all scenarios to finish.
    // A single scenario with 11 replicas took 3-5 seconds.
    // Panic on timeout; works with `cargo nextest` and the `abort_on_panic` above.
    // If we are in the mode where we are looking for faults and `abort_on_panic` is disabled,
    // then this will not have any effect and the simulation will run for as long as it takes to
    // go through all the configured scenarios, and then fail because it didn't panic if no fault was found.
    // If it panics for another reason it might be misleading though, so ideally it should finish early.
    // It would be nicer to actually inspect the panic and make sure it's the right kind of assertion.
    let _guard = zksync_concurrency::testonly::set_timeout(time::Duration::seconds(30));
    // Using `ctc.with_timeout` would stop a runaway execution even without `abort_on_panic` but
    // it would make the test pass for a different reason, not because it found an error but because it ran long.
    let ctx = &ctx::test_root(&ctx::AffineClock::new(10.0));

    #[derive(PartialEq, Debug)]
    struct Replica {
        id: i64, // non-zero ID
        public_key: PublicKey,
        secret_key: SecretKey,
    }

    impl HasKey for Replica {
        type Key = PublicKey;

        fn key(&self) -> &Self::Key {
            &self.public_key
        }
    }

    impl Twin for Replica {
        fn to_twin(&self) -> Self {
            Self {
                id: -self.id,
                public_key: self.public_key.clone(),
                secret_key: self.secret_key.clone(),
            }
        }
    }

    let rng = &mut ctx.rng();

    // The existing test machinery uses the number of finalized blocks as an exit criteria.
    let blocks_to_finalize = 3;
    // The test is going to disrupt the communication by partitioning nodes,
    // where the leader might not be in a partition with enough replicas to
    // form a quorum, therefore to allow N blocks to be finalized we need to
    // go longer.
    let num_rounds = blocks_to_finalize * 10;
    // The paper considers 2 or 3 partitions enough.
    let max_partitions = 3;

    // Every validator has equal power of 1.
    const WEIGHT: u64 = 1;
    let mut spec = SetupSpec::new_with_weights(rng, vec![WEIGHT; num_replicas]);

    let replicas = spec
        .validator_weights
        .iter()
        .enumerate()
        .map(|(i, (sk, _))| Replica {
            id: i as i64 + 1,
            public_key: sk.public(),
            secret_key: sk.clone(),
        })
        .collect::<Vec<_>>();

    let cluster = Cluster::new(replicas, num_twins);
    let scenarios = ScenarioGenerator::<_, NUM_PHASES>::new(&cluster, num_rounds, max_partitions);

    // Gossip with more nodes than what can be faulty.
    let gossip_peers = num_twins + 1;

    // Create network config for all nodes in the cluster (assigns unique network addresses).
    let nets = new_configs_for_validators(
        rng,
        cluster.nodes().iter().map(|r| &r.secret_key),
        gossip_peers,
    );

    let node_to_port = cluster
        .nodes()
        .iter()
        .zip(nets.iter())
        .map(|(node, net)| (node.id, net.server_addr.port()))
        .collect::<HashMap<_, _>>();

    assert_eq!(node_to_port.len(), cluster.num_nodes());

    // Every network needs a behaviour. They are all honest, just some might be duplicated.
    let nodes = vec![(Behavior::Honest, WEIGHT); cluster.num_nodes()];

    // Reuse the same cluster and network setup to run a few scenarios.
    for i in 0..num_scenarios {
        // Generate a permutation of partitions and leaders for the given number of rounds.
        let scenario = scenarios.generate_one(rng);

        // Assign the leadership schedule to the consensus.
        spec.leader_selection =
            LeaderSelectionMode::Rota(scenario.rounds.iter().map(|rc| rc.leader.clone()).collect());

        // Generate a new setup with this leadership schedule.
        let setup = Setup::from(spec.clone());

        // Create a network with the partition schedule of the scenario.
        let splits: PortSplitSchedule = scenario
            .rounds
            .iter()
            .map(|rc| {
                std::array::from_fn(|i| {
                    rc.phase_partitions[i]
                        .iter()
                        .map(|p| p.iter().map(|r| node_to_port[&r.id]).collect())
                        .collect()
                })
            })
            .collect();

        tracing::info!(
            "num_replicas={num_replicas} num_twins={num_twins} num_nodes={} scenario={i}",
            cluster.num_nodes()
        );

        // Debug output of round schedule.
        for (r, rc) in scenario.rounds.iter().enumerate() {
            // Let's just consider the partition of the LeaderCommit phase, for brevity's sake.
            let partitions = &splits[r].last().unwrap();

            let leader_ports = cluster
                .nodes()
                .iter()
                .filter(|n| n.public_key == *rc.leader)
                .map(|n| node_to_port[&n.id])
                .collect::<Vec<_>>();

            let leader_partition_sizes = leader_ports
                .iter()
                .map(|lp| partitions.iter().find(|p| p.contains(lp)).unwrap().len())
                .collect::<Vec<_>>();

            let leader_isolated = leader_partition_sizes
                .iter()
                .all(|s| *s < cluster.quorum_size());

            tracing::info!("round={r} partitions={partitions:?} leaders={leader_ports:?} leader_partition_sizes={leader_partition_sizes:?} leader_isolated={leader_isolated}");
        }

        Test {
            network: Network::Twins(PortRouter::Splits(splits)),
            nodes: nodes.clone(),
            blocks_to_finalize,
        }
        .run_with_config(ctx, nets.clone(), &setup.genesis)
        .await?
    }

    Ok(())
}

/// Test a liveness issue where some validators have the HighQC but don't have the block payload and have to wait for it,
/// while some other validators have the payload but don't have the HighQC and cannot finalize the block, and therefore
/// don't gossip it, which causes a deadlock unless the one with the HighQC moves on and broadcasts what they have, which
/// should cause the others to finalize the block and gossip the payload to them in turn.
#[tokio::test(flavor = "multi_thread")]
async fn test_wait_for_finalized_deadlock() {
    zksync_concurrency::testonly::abort_on_panic();
    let _guard = zksync_concurrency::testonly::set_timeout(time::Duration::seconds(30));
    let ctx = &ctx::test_root(&ctx::AffineClock::new(10.0));

    // These are the conditions for the deadlock to occur:
    // * The problem happens in the handling of LeaderPrepare where the replica waits for the previous block in the justification.
    // * For that the replica needs to receive a proposal from a leader that knows the previous block is finalized.
    // * For that the leader needs to receive a finalized proposal from an earlier leader, but this proposal did not make it to the replica.
    // * Both leaders need to die and never communicate the HighQC they know about to anybody else.
    // * The replica has the HighQC but not the payload, and all other replicas might have the payload, but not the HighQC.
    // * With two leaders down, and the replica deadlocked, we must lose quorum, so the other nodes cannot repropose the missing block either.
    // * In order for 2 leaders to be dow  and quorum still be possible, we need at least 11 nodes.

    // Here are a series of steps to reproduce the issue:
    // 1. Say we have 11 nodes: [0,1,2,3,4,5,6,7,8,9,10], taking turns leading the views in that order; we need 9 nodes for quorum. The first view is view 1 lead by node 1.
    // 2. Node 1 sends LeaderPropose with block 1 to nodes [1-9] and puts together a HighQC.
    // 3. Node 1 sends the LeaderCommit to node 2, then dies.
    // 4. Node 2 sends LeaderPropose with block 2 to nodes [0, 10], then dies.
    // 5. Nodes [0, 10] get stuck processing LeaderPropose because they are waiting for block 1 to appear in their stores.
    // 6. Node 3 cannot gather 9 ReplicaPrepare messages for a quorum because nodes [1,2] are down and [0,10] are blocking. Consensus stalls.

    // To simulate this with the Twins network we need to use a custom routing function, because the 2nd leader mustn't broadcast the HighQC
    // to its peers, but it must receive their ReplicaPrepare's to be able to construct the PrepareQC; because of this the simple split schedule
    // would not be enough as it allows sending messages in both directions.

    let rng = &mut ctx.rng();

    // We want the 2nd proposal to be finalised despite the waiting for block 1.
    let blocks_to_finalize = 2;
    let num_replicas = 11;
    let gossip_peers = 1;

    let mut spec = SetupSpec::new(rng, num_replicas);

    let nodes = spec
        .validator_weights
        .iter()
        .map(|(_, w)| (Behavior::Honest, *w))
        .collect();

    let nets = new_configs_for_validators(
        rng,
        spec.validator_weights.iter().map(|(sk, _)| sk),
        gossip_peers,
    );

    // Assign the validator rota to be in the order of appearance, not ordered by public key.
    spec.leader_selection = LeaderSelectionMode::Rota(
        spec.validator_weights
            .iter()
            .map(|(sk, _)| sk.public())
            .collect(),
    );

    let setup: Setup = spec.into();

    let port_to_id = nets
        .iter()
        .enumerate()
        .map(|(i, net)| (net.server_addr.port(), i))
        .collect::<HashMap<_, _>>();

    // Sanity check the leader schedule
    {
        let pk = setup.genesis.view_leader(ViewNumber(1));
        let cfg = nets
            .iter()
            .find(|net| net.validator_key.as_ref().unwrap().public() == pk)
            .unwrap();
        let port = cfg.server_addr.port();
        assert_eq!(port_to_id[&port], 1);
    }

    let router = PortRouter::Custom(Box::new(move |msg, from, to| {
        use validator::ConsensusMsg::*;
        // Map ports back to logical node ID
        let from = port_to_id[&from];
        let to = port_to_id[&to];
        let view_number = msg.view().number;

        // If we haven't finalised the blocks in the first few rounds, we failed.
        if view_number.0 > 5 {
            return None;
        }

        let can_send = match view_number {
            ViewNumber(1) => {
                match from {
                    // Current leader
                    1 => match msg {
                        // Send the proposal to a subset of nodes
                        LeaderPrepare(_) => to != 0 && to != 10,
                        // Send the commit to the next leader only
                        LeaderCommit(_) => to == 2,
                        _ => true,
                    },
                    // Replicas
                    _ => true,
                }
            }
            ViewNumber(2) => match from {
                // Previous leader is dead
                1 => false,
                // Current leader
                2 => match msg {
                    // Don't send out the HighQC to the others
                    ReplicaPrepare(_) => false,
                    // Send the proposal to the ones which didn't get the previous one
                    LeaderPrepare(_) => to == 0 || to == 10,
                    _ => true,
                },
                // Replicas
                _ => true,
            },
            // Previous leaders dead
            _ => from != 1 && from != 2,
        };

        // eprintln!(
        //     "view={view_number} from={from} to={to} kind={} can_send={can_send}",
        //     msg.label()
        // );

        Some(can_send)
    }));

    Test {
        network: Network::Twins(router),
        nodes,
        blocks_to_finalize,
    }
    .run_with_config(ctx, nets, &setup.genesis)
    .await
    .unwrap()
}
