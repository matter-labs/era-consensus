//! Consensus network is a full graph of connections between all validators.
//! BFT consensus messages are exchanged over this network.
mod handshake;
mod runner;
mod state;
#[cfg(test)]
mod tests;

// Clippy doesn't care about visibility required only for tests.
pub(crate) use runner::*;
#[allow(clippy::wildcard_imports)]
pub use state::Config;
pub(crate) use state::*;
