use crate::{
    chonky_bft::testonly::UTHarness,
    testonly::{
        twins::{Cluster, HasKey, ScenarioGenerator, Twin},
        Behavior, Network, Port, PortRouter, PortSplitSchedule, Test, TestError, NUM_PHASES,
    },
};
use assert_matches::assert_matches;
use std::collections::HashMap;
use test_casing::{cases, test_casing, TestCases};
use zksync_concurrency::{ctx, scope, time};
use zksync_consensus_network::testonly::new_configs_for_validators;
use zksync_consensus_roles::validator::{
    self,
    testonly::{Setup, SetupSpec},
    LeaderSelectionMode, PublicKey, SecretKey, ViewNumber,
};

async fn run_test(behavior: Behavior, network: Network) {
    tokio::time::pause();
    let _guard = zksync_concurrency::testonly::set_timeout(time::Duration::seconds(30));
    zksync_concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);

    const NODES: usize = 11;
    let mut nodes = vec![(behavior, 1u64); NODES];
    // validator::threshold(NODES) will calculate required nodes to validate a message
    // given each node weight is 1
    let honest_nodes_amount = validator::quorum_threshold(NODES as u64) as usize;
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

#[tokio::test]
async fn honest_real_network() {
    run_test(Behavior::Honest, Network::Real).await
}

#[tokio::test]
async fn offline_real_network() {
    run_test(Behavior::Offline, Network::Real).await
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
#[tokio::test]
async fn twins_network_to_fail() {
    tokio::time::pause();
    // With n=5 f=0, so 1 twin means more faulty nodes than expected.
    assert_matches!(
        run_twins(5, 1, TwinsScenarios::Multiple(100)).await,
        Err(TestError::BlockConflict)
    );
}

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
    let _guard = zksync_concurrency::testonly::set_timeout(time::Duration::seconds(60));
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
        .run_with_config(ctx, nets.clone(), &setup)
        .await?
    }

    Ok(())
}

/// Test a liveness issue where some validators have the HighQC but don't have the block payload and have to wait for it,
/// while some other validators have the payload but don't have the HighQC and cannot finalize the block, and therefore
/// don't gossip it, which causes a deadlock unless the one with the HighQC moves on and broadcasts what they have, which
/// should cause the others to finalize the block and gossip the payload to them in turn.
// #[tokio::test]
// async fn test_wait_for_finalized_deadlock() {
//     // These are the conditions for the deadlock to occur:
//     // * The problem happens in the handling of LeaderPrepare where the replica waits for the previous block in the justification.
//     // * For that the replica needs to receive a proposal from a leader that knows the previous block is finalized.
//     // * For that the leader needs to receive a finalized proposal from an earlier leader, but this proposal did not make it to the replica.
//     // * Both leaders need to die and never communicate the HighQC they know about to anybody else.
//     // * The replica has the HighQC but not the payload, and all other replicas might have the payload, but not the HighQC.
//     // * With two leaders down, and the replica deadlocked, we must lose quorum, so the other nodes cannot repropose the missing block either.
//     // * In order for 2 leaders to be dow  and quorum still be possible, we need at least 11 nodes.

//     // Here are a series of steps to reproduce the issue:
//     // 1. Say we have 11 nodes: [0,1,2,3,4,5,6,7,8,9,10], taking turns leading the views in that order; we need 9 nodes for quorum. The first view is view 1 lead by node 1.
//     // 2. Node 1 sends LeaderPropose with block 1 to nodes [1-9] and puts together a HighQC.
//     // 3. Node 1 sends the LeaderCommit to node 2, then dies.
//     // 4. Node 2 sends LeaderPropose with block 2 to nodes [0, 10], then dies.
//     // 5. Nodes [0, 10] get stuck processing LeaderPropose because they are waiting for block 1 to appear in their stores.
//     // 6. Node 3 cannot gather 9 ReplicaPrepare messages for a quorum because nodes [1,2] are down and [0,10] are blocking. Consensus stalls.

//     // To simulate this with the Twins network we need to use a custom routing function, because the 2nd leader mustn't broadcast the HighQC
//     // to its peers, but it must receive their ReplicaPrepare's to be able to construct the PrepareQC; because of this the simple split schedule
//     // would not be enough as it allows sending messages in both directions.

//     // We need 11 nodes so we can turn 2 leaders off.
//     let num_replicas = 11;
//     // Let's wait for the first two blocks to be finalised.
//     // Although theoretically node 1 will be dead after view 1, it will still receive messages and gossip.
//     let blocks_to_finalize = 2;
//     // We need more than 1 gossip peer, otherwise the chain of gossip triggers in the Twins network won't kick in,
//     // and while node 0 will gossip to node 1, node 1 will not send it to node 2, and the test will fail.
//     let gossip_peers = 2;

//     run_with_custom_router(
//         num_replicas,
//         gossip_peers,
//         blocks_to_finalize,
//         |port_to_id| {
//             PortRouter::Custom(Box::new(move |msg, from, to| {
//                 use validator::ConsensusMsg::*;
//                 // Map ports back to logical node ID
//                 let from = port_to_id[&from];
//                 let to = port_to_id[&to];
//                 let view_number = msg.view().number;

//                 // If we haven't finalised the blocks in the first few rounds, we failed.
//                 if view_number.0 > 7 {
//                     return None;
//                 }

//                 // Sending to self is ok.
//                 // If this wasn't here the test would pass even without adding a timeout in process_leader_prepare.
//                 // The reason is that node 2 would move to view 2 as soon as it finalises block 1, but then timeout
//                 // and move to view 3 before they receive any of the ReplicaPrepare from the others, who are still
//                 // waiting to timeout in view 1. By sending ReplicaPrepare to itself it seems to wait or propose.
//                 // Maybe the HighQC doesn't make it from its replica::StateMachine into its leader::StateMachine otherwise.
//                 if from == to {
//                     return Some(true);
//                 }

//                 let can_send = match view_number {
//                     ViewNumber(1) => {
//                         match from {
//                             // Current leader
//                             1 => match msg {
//                                 // Send the proposal to a subset of nodes
//                                 LeaderPrepare(_) => to != 0 && to != 10,
//                                 // Send the commit to the next leader only
//                                 LeaderCommit(_) => to == 2,
//                                 _ => true,
//                             },
//                             // Replicas
//                             _ => true,
//                         }
//                     }
//                     ViewNumber(2) => match from {
//                         // Previous leader is dead
//                         1 => false,
//                         // Current leader
//                         2 => match msg {
//                             // Don't send out the HighQC to the others
//                             ReplicaPrepare(_) => false,
//                             // Send the proposal to the ones which didn't get the previous one
//                             LeaderPrepare(_) => to == 0 || to == 10,
//                             _ => true,
//                         },
//                         // Replicas
//                         _ => true,
//                     },
//                     // Previous leaders dead
//                     _ => from != 1 && from != 2,
//                 };

//                 // eprintln!(
//                 //     "view={view_number} from={from} to={to} kind={} can_send={can_send}",
//                 //     msg.label()
//                 // );

//                 Some(can_send)
//             }))
//         },
//     )
//     .await
//     .unwrap();
// }

/// Run a test with the Twins network controlling exactly who can send to whom in each round.
///
/// The input for the router is a mapping from port to the index of nodes starting from 0.
/// The first view to be executed is view 1 and will have the node 1 as its leader, and so on,
/// so a routing function can expect view `i` to be lead by node `i`, and express routing
/// rules with the logic IDs.
async fn run_with_custom_router(
    num_replicas: usize,
    gossip_peers: usize,
    blocks_to_finalize: usize,
    make_router: impl FnOnce(HashMap<Port, usize>) -> PortRouter,
) -> Result<(), TestError> {
    tokio::time::pause();
    zksync_concurrency::testonly::abort_on_panic();
    let _guard = zksync_concurrency::testonly::set_timeout(time::Duration::seconds(60));
    let ctx = &ctx::test_root(&ctx::RealClock);

    let rng = &mut ctx.rng();

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

    let setup = Setup::from_spec(rng, spec);

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

    Test {
        network: Network::Twins(make_router(port_to_id)),
        nodes,
        blocks_to_finalize,
    }
    .run_with_config(ctx, nets, &setup)
    .await
}
