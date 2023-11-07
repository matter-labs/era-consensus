//! Node role implementation.

mod conv;
mod keys;
mod messages;
mod testonly;

pub use keys::*;
pub use messages::*;
pub use zksync_consensus_schema::proto::roles::node as schema;

#[cfg(test)]
mod tests;
