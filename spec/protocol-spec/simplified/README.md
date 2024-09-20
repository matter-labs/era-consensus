# Methodology for proving safety and liveness

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

In contrast to safety, there is no established methodology for proving liveness
of distributed consensus. This is especially challenging in the case of partial
synchrony. (Proving liveness of distributed consensus in purely synchronous
computations is typically quite easy and in general impossible in purely asynchronous computations due to [FLP85][].)

There are two promising approaches to demonstrating liveness of ChonkyBFT.

## 3.1. At least one block is committed within 5F + 1 views

This is the property offered by Bruno. The good thing is that we can formulate
this properties as a state invariant:

```quint
val block_progress_inv = {
  CORRECT.forall(id => {
    val state = replica_state.get(id)
    state.view < (5 * F + 1) * (state.committed_blocks.length() + 1)
  })
}
```

To find a shortest counterexample to `block_progress_inv`, we would need an
execution, in which at least one replica switches six views. We estimate such an
execution to have at least 60 steps. This is beyond the reach of bounded model
checking and randomized simulation of ChonkyBFT, which we have performed
earlier.

Hence, similar to demonstrating `agreement_inv`, we should be able to use the
inductive invariant to show that `block_progress_inv` is not violated.  However,
this additional invariant may require us to improve the inductive invariant.

## 3.2. Prophecy variable with a magic round

Some proofs of distributed consensus include the notion of a "magic round".  It
is the round, where a correct replica makes a decision. It is often the case
that all correct replicas have to make the same decision in the magic round or
in the next round.

Similar to that idea, Giuliano Losa has recently used a prophecy variable in
his [liveness proof of TetraBFT][tetrabft-liveness] with Apalache.

The idea is as follows:

 - Non-deterministically choose a "good ballot" in the initial state.  A good
 ballot would correspond to a "good view" in ChonkyBFT.
 
 - Once a replica reaches the good ballot, it is not allowed to progress to the
 next rounds/views. Thus, the execution space is restricted to the scope bounded
 by the good ballot.
 
 - Check the following state invariant. If all correct replicas have reached the
 good ballot/view, and they cannot execute further actions in this view, one of
 them *must* commit a block. See the [invariant][tetratla-liveness] in the
 TLA<sup>+</sup> specification.

It looks like there is a relation between the both approaches. Intuitively, the
magic ballot corresponds to the view that works under GST. Hence, if replicas
may receive the sent messages, they must receive this messages. This corresponds
to the expectations of partial synchrony.


[n6f1b1_inductive.qnt]: ./n6f1b1_inductive.qnt
[blogpost]: https://protocols-made-fun.com/consensus/matterlabs/quint/specification/modelchecking/2024/07/29/chonkybft.html
[Apalache]: https://github.com/apalache-mc/apalache
[tendermint-inductive]: https://github.com/cometbft/cometbft/blob/main/spec/light-client/accountability/TendermintAccInv_004_draft.tla
[FLP85]: https://dl.acm.org/doi/10.1145/3149.214121
[tetrabft-liveness]: https://github.com/nano-o/tetrabft-tla/tree/main
[tetratla-liveness]: https://github.com/nano-o/tetrabft-tla/blob/91916dfca49a5d59809212c1687b5680e0c98270/TetraBFT.tla#L227