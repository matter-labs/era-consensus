# ChonkyBFT Informal Specification

This is the ChonkyBFT specification in pseudocode.

We’ll assume there’s a static set of nodes. Each node has 3 components: replica, proposer and fetcher. They are modeled as concurrent tasks or actors. Proposer and fetcher can read the replica state, but can’t write to it.

There's a couple of considerations that are not described in the pseudo-code:

- **Network model**. We use a stronger form of the partially synchronous model. Up until GST, messages can be delayed, delivered out of order or even _dropped_ altogether. After GST the network becomes synchronous.
- **Garbage collection**. We can’t store all messages, the goal here is to bound the number of messages that each replica stores, in order to avoid DoS attacks. We handle messages like this:
    - `NewView` messages are never stored, so no garbage collection is necessary.
    - We keep all `Proposal` messages until the proposal (or a proposal with the same block number) is finalized (which means any honest replica having both the `Proposal` and the corresponding `CommitQC`, we assume that any honest replica in that situation will immediately broadcast the block on the p2p network.
    - We only store the newest `CommitVote` **and** `TimeoutVote` for each replica. Honest replicas only change views on QCs, so if they send a newer message, they must also have sent a `NewView` on the transition, which means we can just get the QC from that replica. Even if the other replicas don’t receive the QC, it will just trigger a reproposal.