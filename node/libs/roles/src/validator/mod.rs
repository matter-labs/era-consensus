//! Validator role implementation.

#[cfg(test)]
mod tests;

mod conv;
mod keys;
mod messages;
mod testonly;

pub use keys::*;
pub use messages::*;
pub use schema::proto::roles::validator as schema;
