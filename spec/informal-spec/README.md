# ChonkyBFT Specification

This is a ChonkyBFT specification in pseudocode.

There's a couple of considerations that are not described in the pseudo-code:

- **Network model**. Messages might be delivered out of order, but we don’t guarantee eventual delivery for *all* messages. Actually, our network only guarantees eventual delivery of the most recent message for each type. That’s because each replica only stores the last outgoing message of each type in memory, and always tries to deliver those messages whenever it reconnects with another replica.
- **Garbage collection**. We can’t store all messages, the goal here is to bound the number of messages that each replica stores, in order to avoid DoS attacks. We handle messages like this:
    - `NewView` messages are never stored, so no garbage collection is necessary.
    - We keep all `Proposal` messages until the proposal (or a proposal with the same block number) is finalized (which means any honest replica having both the `Proposal` and the corresponding `CommitQC`, we assume that any honest replica in that situation will immediately broadcast the block on the gossip network).
    - We only store the newest `CommitVote` **and** `TimeoutVote` for each replica. Honest replicas only change views on QCs, so if they send a newer message, they must also have sent a `NewView` on the transition, which means we can just get the QC from that replica. Even if the other replicas don’t receive the QC, it will just trigger a reproposal.