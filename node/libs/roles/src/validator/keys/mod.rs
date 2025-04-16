//! Cryptographic keys representing the validator role.

mod aggregate_signature;
mod proof_of_possession;
mod public_key;
mod secret_key;
mod signature;
mod testonly;
#[cfg(test)]
mod tests;

pub use aggregate_signature::AggregateSignature;
pub use proof_of_possession::ProofOfPossession;
pub use public_key::PublicKey;
pub use secret_key::SecretKey;
pub use signature::Signature;
