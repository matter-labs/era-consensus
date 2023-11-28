//! Implements the leader role in the Fastest-HotStuff consensus algorithm. The leader is the role that proposes blocks
//! and aggregates replica messages. It mainly acts as a central point of communication for the replicas. Note that
//! our consensus node will perform both the replica and leader roles simultaneously.

mod replica_commit;
mod replica_prepare;
mod state_machine;

pub(crate) use state_machine::StateMachine;

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) use replica_commit::Error as ReplicaCommitError;
#[cfg(test)]
pub(crate) use replica_prepare::Error as ReplicaPrepareError;
