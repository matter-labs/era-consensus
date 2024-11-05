//! Cryptographic keys representing the validator role.

mod aggregate_signature;
mod public_key;
mod secret_key;
mod signature;
#[cfg(test)]
mod tests;

pub use aggregate_signature::AggregateSignature;
pub use public_key::PublicKey;
pub use secret_key::SecretKey;
pub use signature::{ProofOfPossession, Signature};
