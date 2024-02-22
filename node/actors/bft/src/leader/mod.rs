//! Implements the leader role in the Fastest-HotStuff consensus algorithm. The leader is the role that proposes blocks
//! and aggregates replica messages. It mainly acts as a central point of communication for the replicas. Note that
//! our consensus node will perform both the replica and leader roles simultaneously.

pub(crate) mod replica_commit;
pub(crate) mod replica_prepare;
mod state_machine;
#[cfg(test)]
mod tests;

pub(crate) use self::state_machine::StateMachine;
