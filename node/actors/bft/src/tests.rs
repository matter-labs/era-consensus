use std::collections::HashMap;

use crate::testonly::{
    twins::{Cluster, HasKey, ScenarioGenerator, Twin},
    ut_harness::UTHarness,
    Behavior, Network, PortSplitSchedule, Test,
};
use zksync_concurrency::{
    ctx::{self, Ctx},
    scope, time,
};
use zksync_consensus_network::testonly::new_configs_for_validators;
use zksync_consensus_roles::validator::{
    self,
    testonly::{Setup, SetupSpec},
    LeaderSelectionMode, PublicKey, SecretKey,
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
    let ctx = &ctx::test_root(&ctx::AffineClock::new(10.0));
    // n<6 implies f=0 and q=n
    run_twins(ctx, 5, 0, 10).await.unwrap();
}

/// Run Twins scenarios without actual twins, but enough replicas that partitions
/// can play a role, isolating certain nodes (potentially the leader) in some
/// rounds.
///
/// This should be a sanity check that without Byzantine behaviour the consensus
/// is resilient to temporary network partitions.
#[tokio::test(flavor = "multi_thread")]
async fn twins_network_wo_twins_w_partitions() {
    let ctx = &ctx::test_root(&ctx::AffineClock::new(10.0));
    // n=6 implies f=1 and q=5; 6 is the minimum where partitions are possible.
    run_twins(ctx, 6, 0, 5).await.unwrap();
}

/// Run Twins scenarios with random number of nodes and 1 twin.
#[tokio::test(flavor = "multi_thread")]
async fn twins_network_w1_twins_w_partitions() {
    let ctx = &ctx::test_root(&ctx::AffineClock::new(10.0));
    // n>=6 implies f>=1 and q=n-f
    for num_replicas in 6..=10 {
        // let num_honest = validator::threshold(num_replicas as u64) as usize;
        // let max_faulty = num_replicas - num_honest;
        // let num_twins = rng.gen_range(1..=max_faulty);
        run_twins(ctx, num_replicas, 1, 3).await.unwrap();
    }
}

/// Run Twins scenarios with higher number of nodes and 2 twins.
#[tokio::test(flavor = "multi_thread")]
async fn twins_network_w2_twins_w_partitions() {
    let ctx = &ctx::test_root(&ctx::AffineClock::new(10.0));
    // n>=11 implies f>=2 and q=n-f
    // TODO: This fails for now.
    run_twins(ctx, 11, 2, 1).await.unwrap();
}

/// Create network configuration for a given number of replicas and twins and run [Test].
async fn run_twins(
    ctx: &Ctx,
    num_replicas: usize,
    num_twins: usize,
    num_scenarios: usize,
) -> anyhow::Result<()> {
    zksync_concurrency::testonly::abort_on_panic();
    // Use a single timeout for all scenarios to finish.
    // A single scenario with 11 replicas took 3-5 seconds.
    let _guard = zksync_concurrency::testonly::set_timeout(time::Duration::seconds(30));

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
    let scenarios = ScenarioGenerator::new(&cluster, num_rounds, max_partitions);

    // Create network config for all nodes in the cluster (assigns unique network addresses).
    let nets = new_configs_for_validators(rng, cluster.nodes().iter().map(|r| &r.secret_key), 1);

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
                rc.partitions
                    .iter()
                    .map(|p| p.iter().map(|r| node_to_port[&r.id]).collect())
                    .collect()
            })
            .collect();

        eprintln!(
            "num_replicas={num_replicas} num_twins={num_twins} num_nodes={} scenario={i}",
            cluster.num_nodes()
        );

        for (r, rc) in scenario.rounds.iter().enumerate() {
            let leader_id = cluster
                .nodes()
                .iter()
                .find(|n| n.public_key == *rc.leader)
                .unwrap()
                .id;
            let leader_port = node_to_port[&leader_id];
            let partitions = &splits[r];
            let leader_partition_size = partitions
                .iter()
                .find(|p| p.contains(&leader_port))
                .unwrap()
                .len();
            let leader_isolated = leader_partition_size < cluster.quorum_size();

            eprintln!("round={r} partitions={partitions:?} leader={leader_port} leader_partition_size={leader_partition_size} leader_isolated={leader_isolated}");
        }

        Test {
            network: Network::Twins(splits),
            nodes: nodes.clone(),
            blocks_to_finalize,
        }
        .run_with_config(ctx, nets.clone(), &setup.genesis)
        .await?
    }

    Ok(())
}
