//! Implements the replica role in the Fastest-HotStuff consensus algorithm. The replica is the role that validates
//! proposals, votes for them and finalizes them. It basically drives the consensus forward. Note that our consensus
//! node will perform both the replica and leader roles simultaneously.

mod block;
pub(crate) mod leader_commit;
pub(crate) mod leader_prepare;
mod new_view;
pub(crate) mod replica_prepare;
mod state_machine;
#[cfg(test)]
mod tests;
mod timer;

pub(crate) use self::state_machine::StateMachine;
