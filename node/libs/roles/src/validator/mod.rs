//! Validator role implementation.

#[cfg(test)]
mod tests;

mod conv;
mod keys;
mod messages;
pub mod testonly;

pub use self::{keys::*, messages::*};
// TODO(gprusak): it should be ok to have an unsigned
// genesis. For now we need a way to bootstrap the chain.
pub use testonly::GenesisSetup;
