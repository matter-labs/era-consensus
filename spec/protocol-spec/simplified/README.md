# Methodology of proving safety and liveness

This is a draft document that captures our approach to demonstrating safety and
liveness of the simplified version of ChonkyBFT under bounded inputs.

## 1. Main assumptions

We fix the protocol parameters to small but reasonable values, e.g., as in
[n6f1b1_inductive.qnt][]:

   - 5 correct replicas
   - 1 Byzantine replica
   - up to 7 views in each correct replica
   - up to 2 blocks committed by each correct replica
   - up to 2 valid block hashes (that may appear in multiple messages)
   - up to 1 invalid block hash (that may appear in multiple messages)
   - up to 6 commit certificates
   - up to 6 timeout certificates
   - up to 20 timeout messages

The choice of these parameters is dictated by the nature of the protocol.  Since
replicas are exchanging with quorum certificates, which may be nested, the
potential number of different messages in the protocol is growing rapidly, even
though correct replicas are sending a small number of messages in a single view.

By checking the properties for small values, we can iterate faster without
waiting for counterexamples from the model checker for days.

## 2. Demonstrating safety

As we discussed in the [blogpost][], we have checked a number of state
invariants via bounded model checking and randomized symbolic execution. Both of
these techniques explore symbolic executions up to a certain length, typically,
up to 20-30 steps. Scaling the experiments to a significantly larger number of
steps for ChonkyBFT does not seem possible in the short term. 

As a result, to show safety for arbitrarily long executions (under the bounded
parameters), we use the inductive approach. Specifically, we want to show that
the state invariant `agreement_inv` holds true, if the number of faults is
within the expected bounds. We call this invariant $A$.  In a nutshell, the idea
is to find a predicate $IndInv$ that has three properties (in the notation of
TLA<sup>+</sup>):

 1. All initial states satisfy the inductive invariant: $Init \Rightarrow IndInv$.
 
 1. If a state satisfies the inductive invariant, it does not violate the state
 invariant $A$. That is, $IndInv \Rightarrow A$.
 
 1. If a state satisfies the inductive invariant, a single protocol step cannot
    produce a state that violates the inductive invariant. That is,
    $IndInv \land Next \Rightarrow Inv'$.

This is the standard approach of proving protocol safety with TLA<sup>+</sup>
and its proof system, which were introduced by Leslie Lamport. This is also the
standard approach of proving safety with symbolic model checkers. In particular,
[Apalache][] supports this approach, albeit for bounded inputs. For instance, we
constructed an [inductive invariant of Tendermint][tendermint-inductive] some
time ago.

We adapt this approach to the Quint specification of ChonkyBFT as follows:

 - The inductive invariant `ind_inv` is written as a conjunction of lemmas,
   which are simply smaller invariants.

 - As we have to bound the scope, we cannot simply start in a state that
 satisfies the invariant. Instead, we define the action `ind_init` that
 non-deterministically initializes state variables. Further, we restrict the
 states with `ind_inv` in `post_ind_inv` and `step`.

Hence, we run the following model checking queries with `quint verify`:

 1. Checking the initial states:
 
    ```sh
    $ quint verify --apalache-config=apalache-inductive.json \
      --max-steps=1 --step=post_ind_inv \
      --invariant=agreement_inv n6f1b1_inductive.qnt
    ```

 1. Checking agreement against the inductive invariant:
 
    ```sh
    $ quint verify --apalache-config=apalache-inductive.json \
      --max-steps=1 --init=ind_init --step=post_ind_inv \
      --invariant=agreement_inv n6f1b1_inductive.qnt
    ```

 1. Checking the inductive step:
 
    ```sh
    $ quint verify --apalache-config=apalache-inductive.json \
      --max-steps=1 --init=ind_init  --invariant=ind_inv \
      n6f1b1_inductive.qnt
    ```
 
## 3. Demonstrating liveness


[n6f1b1_inductive.qnt]: ./n6f1b1_inductive.qnt
[blogpost]: https://protocols-made-fun.com/consensus/matterlabs/quint/specification/modelchecking/2024/07/29/chonkybft.html
[Apalache]: https://github.com/apalache-mc/apalache
[tendermint-inductive]: https://github.com/cometbft/cometbft/blob/main/spec/light-client/accountability/TendermintAccInv_004_draft.tla