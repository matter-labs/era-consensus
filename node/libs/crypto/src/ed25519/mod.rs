//! ed25519 signature scheme.
//! This is just an adapter of ed25519_dalek, exposing zksync-bft-specific API.

use crate::ByteFmt;
use anyhow::Context as _;
use ed::{Signer as _, Verifier as _};
use ed25519_dalek as ed;

pub mod testonly;

/// ed25519 secret key.
pub struct SecretKey(ed::SigningKey);

impl SecretKey {
    /// Generates a secret key from a cryptographically-secure entropy source.
    pub fn generate() -> Self {
        Self(ed::SigningKey::generate(&mut rand::rngs::OsRng {}))
    }

    /// Signs a message.
    pub fn sign(&self, msg: &[u8]) -> Signature {
        // If this turns out to be inefficient, we can cache the expanded
        // secret key together with the secret key.
        Signature(self.0.sign(msg))
    }

    /// Computes a public key for this secret key.
    pub fn public(&self) -> PublicKey {
        // If this turns out to be inefficient, we can cache the public key
        // together with the secret key.
        PublicKey(ed::VerifyingKey::from(&self.0))
    }
}

impl ByteFmt for SecretKey {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        let bytes: &ed::SecretKey = bytes.try_into()?;
        Ok(Self(ed::SigningKey::from_bytes(bytes)))
    }

    fn encode(&self) -> Vec<u8> {
        self.0.to_bytes().to_vec()
    }
}

/// ed25519 public key.
#[derive(Clone)]
pub struct PublicKey(ed::VerifyingKey);

impl PublicKey {
    /// Verifies a signature of a message against this public key.
    pub fn verify(&self, msg: &[u8], sig: &Signature) -> Result<(), InvalidSignatureError> {
        self.0
            .verify(msg, &sig.0)
            .map_err(|_| InvalidSignatureError)
    }
}

impl ByteFmt for PublicKey {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        let bytes: &[u8; ed::PUBLIC_KEY_LENGTH] = bytes.try_into()?;
        ed::VerifyingKey::from_bytes(bytes)
            .context("invalid key material")
            .map(Self)
    }

    fn encode(&self) -> Vec<u8> {
        self.0.as_bytes().to_vec()
    }
}

impl std::hash::Hash for PublicKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write(self.0.as_bytes());
    }
}

// clippy: if Hash is implemented manually,
// then PartialEq should be implemented manually.
impl PartialEq for PublicKey {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq(&other.0)
    }
}

impl Eq for PublicKey {}

impl PartialOrd for PublicKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PublicKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.as_bytes().cmp(other.0.as_bytes())
    }
}

/// ed25519 signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature(ed::Signature);

impl ByteFmt for Signature {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        let bytes: &[u8; ed::SIGNATURE_LENGTH] = bytes.try_into()?;
        Ok(Self(ed::Signature::from_bytes(bytes)))
    }

    fn encode(&self) -> Vec<u8> {
        self.0.to_bytes().to_vec()
    }
}

/// Error returned when an invalid signature is detected.
#[derive(Debug, thiserror::Error)]
#[error("invalid signature")]
pub struct InvalidSignatureError;
