//! Keys and signatures used by the attester.

#[allow(unused)] // TODO: Remove once EIP-2537 is merged and we switch to BLS.
mod aggregate_signature;
mod public_key;
mod secret_key;
mod signature;

pub use aggregate_signature::{AggregateMultiSig, AggregateSignature};
pub use public_key::PublicKey;
pub use secret_key::SecretKey;
pub use signature::{MultiSig, Signature};
