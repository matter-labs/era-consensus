//! Cryptographic keys representing the validator role.

mod aggregate_signature;
mod public_key;
mod secret_key;
mod signature;

pub use aggregate_signature::AggregateSignature;
pub use public_key::PublicKey;
pub use secret_key::SecretKey;
pub use signature::Signature;

/// Error type returned by validator key operations.
pub type Error = zksync_consensus_crypto::bls12_381::Error;
