//! Implements the replica role in the Fastest-HotStuff consensus algorithm. The replica is the role that validates
//! proposals, votes for them and finalizes them. It basically drives the consensus forward. Note that our consensus
//! node will perform both the replica and leader roles simultaneously.

mod block;
mod leader_commit;
mod leader_prepare;
mod new_view;
mod state_machine;
#[cfg(test)]
mod tests;
mod timer;

pub(crate) use state_machine::StateMachine;
