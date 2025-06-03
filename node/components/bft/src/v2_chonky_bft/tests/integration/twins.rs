use std::collections::HashMap;

use assert_matches::assert_matches;
use test_casing::{cases, test_casing, TestCases};
use zksync_concurrency::{ctx, time};
use zksync_consensus_network::testonly::new_configs_for_validators;
use zksync_consensus_roles::validator::{
    testonly::{Setup, SetupSpec},
    ProtocolVersion, PublicKey, SecretKey, ViewNumber,
};

use crate::{
    testonly::{
        twins::{Cluster, HasKey, ScenarioGenerator, Twin},
        Behavior,
    },
    v2_chonky_bft::testonly::{
        IntegrationTestConfig, PortRouter, PortSplitSchedule, TestError, TestNetwork, NUM_PHASES,
    },
};

/// Govern how many scenarios to execute in the test.
enum TwinsScenarios {
    /// Execute N scenarios in a loop.
    ///
    /// Use this when looking for a counter example, ie. a scenario where consensus fails.
    Multiple(usize),
    /// Execute 1 scenario after doing N reseeds of the RNG.
    ///
    /// Use this with the `#[test_casing]` macro to turn scenarios into separate test cases.
    Reseeds(usize),
}

/// Create network configuration for a given number of replicas and twins and run [Test],
async fn run_twins(
    num_replicas: usize,
    num_twins: usize,
    scenarios: TwinsScenarios,
) -> Result<(), TestError> {
    zksync_concurrency::testonly::abort_on_panic();

    // A single scenario with 11 replicas took 3-5 seconds.
    // Panic on timeout; works with `cargo nextest` and the `abort_on_panic` above.
    let _guard = zksync_concurrency::testonly::set_timeout(time::Duration::seconds(90));
    let ctx = &ctx::test_root(&ctx::RealClock);

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

    let (num_scenarios, num_reseeds) = match scenarios {
        TwinsScenarios::Multiple(n) => (n, 0),
        TwinsScenarios::Reseeds(n) => (1, n),
    };

    // Keep scenarios separate by generating a different RNG many times.
    let mut rng = ctx.rng();
    for _ in 0..num_reseeds {
        rng = ctx.rng();
    }
    let rng = &mut rng;

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
    let spec = SetupSpec::new_with_weights_and_version(
        rng,
        vec![WEIGHT; num_replicas],
        ProtocolVersion(2),
    );

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
        // Generate a permutation of partitions for the given number of rounds.
        let scenario = scenarios.generate_one(rng);

        // Generate a new setup with this leadership schedule.
        let setup = Setup::from_spec(rng, spec.clone());

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

        tracing::trace!(
            "num_replicas={num_replicas} num_twins={num_twins} num_nodes={} scenario={i}",
            cluster.num_nodes()
        );

        // Debug output of round schedule.
        #[allow(clippy::needless_range_loop)]
        for r in 0..scenario.rounds.len() {
            // Let's just consider the partition of the LeaderCommit phase, for brevity's sake.
            let partitions = &splits[r].last().unwrap();

            let leader_pk = setup
                .validators_schedule()
                .view_leader(ViewNumber(r as u64));

            let leader_ports = cluster
                .nodes()
                .iter()
                .filter(|n| n.public_key == leader_pk)
                .map(|n| node_to_port[&n.id])
                .collect::<Vec<_>>();

            let leader_partition_sizes = leader_ports
                .iter()
                .map(|lp| partitions.iter().find(|p| p.contains(lp)).unwrap().len())
                .collect::<Vec<_>>();

            let leader_isolated = leader_partition_sizes
                .iter()
                .all(|s| *s < cluster.quorum_size());

            tracing::trace!(
                "round={r} partitions={partitions:?} leaders={leader_ports:?} \
                 leader_partition_sizes={leader_partition_sizes:?} \
                 leader_isolated={leader_isolated}"
            );
        }

        IntegrationTestConfig {
            network: TestNetwork::Twins(PortRouter::Splits(splits)),
            nodes: nodes.clone(),
            blocks_to_finalize,
        }
        .run_with_config(ctx, nets.clone(), &setup)
        .await?
    }

    Ok(())
}

/// Run Twins scenarios without actual twins, and with so few nodes that all
/// of them are required for a quorum, which means (currently) there won't be
/// any partitions.
///
/// This should be a simple sanity check that the network works and consensus
/// is achieved under the most favourable conditions.
#[test_casing(10,0..10)]
#[tokio::test]
async fn twins_network_wo_twins_wo_partitions(num_reseeds: usize) {
    tokio::time::pause();
    // n<6 implies f=0 and q=n
    run_twins(5, 0, TwinsScenarios::Reseeds(num_reseeds))
        .await
        .unwrap();
}

/// Run Twins scenarios without actual twins, but enough replicas that partitions
/// can play a role, isolating certain nodes (potentially the leader) in some
/// rounds.
///
/// This should be a sanity check that without Byzantine behaviour the consensus
/// is resilient to temporary network partitions.
#[test_casing(5,0..5)]
#[tokio::test]
async fn twins_network_wo_twins_w_partitions(num_reseeds: usize) {
    tokio::time::pause();
    // n=6 implies f=1 and q=5; 6 is the minimum where partitions are possible.
    run_twins(6, 0, TwinsScenarios::Reseeds(num_reseeds))
        .await
        .unwrap();
}

/// Test cases with 1 twin, with 6-10 replicas, 10 scenarios each.
const CASES_TWINS_1: TestCases<(usize, usize)> = cases! {
    (6..=10).flat_map(|num_replicas| (0..10).map(move |num_reseeds| (num_replicas, num_reseeds)))
};

/// Run Twins scenarios with random number of nodes and 1 twin.
#[test_casing(50, CASES_TWINS_1)]
#[tokio::test]
async fn twins_network_w1_twins_w_partitions(num_replicas: usize, num_reseeds: usize) {
    tokio::time::pause();
    // n>=6 implies f>=1 and q=n-f
    // let num_honest = validator::threshold(num_replicas as u64) as usize;
    // let max_faulty = num_replicas - num_honest;
    // let num_twins = rng.gen_range(1..=max_faulty);
    run_twins(num_replicas, 1, TwinsScenarios::Reseeds(num_reseeds))
        .await
        .unwrap();
}

/// Run Twins scenarios with higher number of nodes and 2 twins.
#[test_casing(5,0..5)]
#[tokio::test]
async fn twins_network_w2_twins_w_partitions(num_reseeds: usize) {
    tokio::time::pause();
    // n>=11 implies f>=2 and q=n-f
    run_twins(11, 2, TwinsScenarios::Reseeds(num_reseeds))
        .await
        .unwrap();
}

/// Run Twins scenario with more twins than tolerable and expect it to fail.
/// Disabled by default since it can take a long time to run depending on
/// the initial seed.
#[ignore]
#[tokio::test]
async fn twins_network_to_fail() {
    tokio::time::pause();
    assert_matches!(
        // All twins! To find a conflict quicker.
        run_twins(6, 6, TwinsScenarios::Multiple(200)).await,
        Err(TestError::BlockConflict)
    );
}
