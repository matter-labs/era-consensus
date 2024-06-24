# ChonkyBFT

This page summarizes the scope of the Quint specification and the experiments we
have done with it. This Quint specification was prepared by Igor Konnov and
Denis Kolegov (Matter Labs) under a contract with Matter Labs.

We present the experiments with randomized simulation and bounded model
checking. An inductive proof of safety is work in progress.

## 1. Protocol configurations

We are checking the protocol specification for several configurations:

 - [n6f1b0](./n6f1b0.qnt) is the protocol instance for `N=6`, `F=1`, and no
 faulty replicas. All six replicas are correct, though some of them may be slow.
 This instance must be safe, e.g., `agreement_inv` must not be violated.

 - [n7f1b0](./n7f1b0.qnt) is the protocol instance for `N=7`, `F=1`, and no
 faulty replicas. All seven replicas are correct, though some of them may be slow.
 This instance lets us test whether more correct replicas may introduce issues.

 - [n6f1b1](./n6f1b1.qnt) is the protocol instance for `N=6`, `F=1`, and one
 Byzantine replica. Five correct replicas should be able to tolerate one
 Byzantine replica. This instance must be safe, e.g., `agreement_inv` must not
 be violated.
 
 - [n6f1b2](./n6f1b2.qnt) is the protocol instance for `N=6`, `F=1`, and two
 Byzantines replica. Two Byzantine replicas have the critical mass to partition
 the correct replicas. Some invariants such as `agreement_inv` are violated in
 this instance.
 
 - [n6f1b3](./n6f1b3.qnt) is the protocol instance for `N=6`, `F=1`, and three
 Byzantines replica. Many invariants are violated in this instance very quickly.
 
## 2. Finding examples

It is usually a good idea to find examples of the states that you believe the
protocol should be able to reach. We produce such examples by checking "falsy"
state invariants, which carry the suffix `_example`:

|#| Invariant        | Description                                                   |
|--| ---------------- | ------------------------------------------------------------- |
|1| `phase_commit_example` | Produce an example of a correct replica reaching `PhaseCommit` |
|2| `high_timeout_qc_example` | Produce an example of a non-empty `TimeoutQC` being stored as a high `TimeoutQC` by a correct replica |
|3| `timeout_qc_example` | Produce an execution that lets us to form a `TimeoutQC` |
|4| `commit_qc_example` | Produce an execution that lets us to form a `CommitQC` |
|5| `justification_example` | Produce an execution that produces at least one justification in a non-zero view |
|6| `high_commit_qc_example` | Produce an execution, where a correct replica stores a high `CommitQC` |
|7| `views_over_2_example` | Produce an execution, where a correct replica progresses beyond view 2 |
|8| `blocks_example` | Produce an execution, where a correct replica finalizes one block |
|9| `no_block_gaps_example` | Produce an execution, where a correct replica contains gaps in its finalized blocks |
|10| `two_blocks_example` | Produce an execution, where two correct replicas commit two different blocks (e.g., in different views) |
|11| `two_chained_blocks_example` | Produce an execution, where a correct replica finalizes two blocks |

There are two ways to find examples, which we consider below.

**Running the randomized simulator.** With this command, the tool is simply replacing non-deterministic choices such as `any` and `oneOf` with random choice. For simple invariants, the randomized simulator produces examples very quickly:

  ```
  $ quint run --invariant=phase_commit_example ./experiments/n6f1b0.qnt
  ```

**Running randomized symbolic execution.** With this command, the tool is randomly choosing
actions in `any {...}` but keeps the choice of data in `oneOf` symbolic. It further offloads the task of finding an execution to the symbolic model checker called [Apalache][]. This is how we run symbolic
execution:

  ```
  $ quint verify --random-transitions=true --invariant=phase_commit_example ./experiments/n6f1b0.qnt
  ```

## 3. State invariants

In the course of writing the specification, we have developed a number of state
invariants. By simulating and model checking the specification against these
invariants increases our confidence in the correctness of the protocol.

|#| Invariant        | Description                                                   |
|--| ---------------- | ------------------------------------------------------------- |
|1| `agreement_inv`  | No two correct replicas finalize two different blocks for the same block number |
|2| `global_blockchain_inv` | The states of all correct replicas allow us to build a blockchain |
|3| `committed_blocks_have_justification_inv` | Every block finalized by a correct replica is backed by a justification |
|4| `no_proposal_equivocation_inv` | No correct proposer sends two different proposals in the same view |
|5| `no_commit_equivocation_inv` | No correct proposer sends two different commit votes in the same view |
|6| `no_timeout_equivocation_inv` | No correct proposer sends two different timeout votes in the same view |
|7| `no_new_view_equivocation_inv` | No correct proposer sends two different `NewView` messages in the same view |
|8| `store_signed_commit_all_inv` | Integrity of the signed commit votes that are stored by the correct replicas |
|9| `store_signed_timeout_all_inv` | Integrity of the signed timeout votes that are stored by the correct replicas |
|10| `view_justification_inv` | Every view change is backed by a justification |
|11| `justification_is_supported_inv` | Every justification stored by a correct replica is backed by messages that were sent by other replicas |
|12| `one_high_vote_in_timeout_qc_inv` | A single `TimeoutQC` does not let us form two high votes |
|13| `one_commit_quorum_inv` | It should not be possible to build two quorums for two different block hashes in the same view |
|14| `all_replicas_high_commit_qc_inv` | Integrity of high `CommitQC`'s stored by correct replicas |
|15| `all_replicas_high_timeout_qc_inv` | Integrity of high `TimeoutQC`'s stored by correct replicas |
|16| `msgs_signed_timeout_inv` | Correct replicas do not send malformed timeout votes |
|17| `msgs_signed_commit_inv` | Correct replicas do not send malformed commit votes |
|18| `all_replicas_high_vote_inv` | Correct replicas do not store malformed high votes |
|19| `timeout_high_vote_is_highest_inv` | The high vote is the highest among the votes sent by a replica |

## 4. Checking state invariants

**Randomized simulator.** Similar to finding examples, we run the randomized simulator
to check correctness of the protocol against state invariants. For example:

```sh
quint run --max-steps=20 --max-samples=100000 --invariant=no_commit_equivocation_inv n6f1b3.qnt
```

By changing `--max-steps`, we give the simulator how deep the executions should be, e.g., up to 20 steps. By changing `--max-samples`, we give the simulator the number of executions to try, e.g., 100,000.

**Randomized symbolic execution.** Symbolic execution offloads the task of
finding an execution to the symbolic model checker, while randomly choosing
actions at each step:

```sh
quint run --max-steps=20 --random-transitions=true --invariant=no_commit_equivocation_inv n6f1b3.qnt
```

Note that there is no need to specify `--max-samples`, as the symbolic model
checker does not enumerate concrete executions, in contrast to the randomized
simulator. Instead, the symbolic model checker enumerates up to 100 *symbolic
executions*.

**Bounded model checker.** The bounded model checker checks an invariant against
*all* executions up to given depth:

```sh
quint run --max-steps=20 --invariant=no_commit_equivocation_inv n6f1b3.qnt
```

Bounded model checker gives us the most correctness guarantees. These guarantees
come with the price of this command being the slowest one.

## 5. Checking invariants in parallel

Randomized simulations and model checking may take quite considerable time. This
is not surprising, as our task is no easier than fuzzing. To utilize multi-core
CPUs and beefy machines, we introduce two parallelization scripts. Both scripts
require [GNU Parallel][].

**Script quint-verify-parallel.sh** This script lets us run randomized symbolic
execution across multiple nodes. Since actions are chosen at random, we explore
multiple symbolic paths in parallel. Here is how it is run:

```sh
./experiments/quint-verify-parallel.sh n6f1b3.qnt all_invariants 30 800 7
```

Type `./experiments/quint-verify-parallel.sh` to read about the meaning of the command-line arguments.

This command finishes as soon as one of the jobs finds an invariant violation.
You can check the logs in the directory called `./out`.

**Script test-invariants.sh** This script parallelizes invariant checking across
multiple CPUs:

```sh
./experiments/test-invariants.sh n6f1b1.qnt all-invariants.csv 30 12 100
```

Type `./experiments/test-invariants.sh` to read about the meaning of the
command-line arguments. The most important argument is the
comma-separated-values (CSV) file that contains the invariants to check and the
server ports to use when running the model checker.

This script does not terminate immediately, when an invariant has been violated.
Rather, it keeps running, in order to find more violations.


[Apalache]: https://github.com/informalsystems/apalache/
[GNU Parallel]: https://www.gnu.org/software/parallel/