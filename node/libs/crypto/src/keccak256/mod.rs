//! Wrappers for the Keccak256 cryptographic hash algorithm.
use crate::ByteFmt;
use sha3::{digest::Update as _, Digest as _};

mod test;
pub mod testonly;

/// Keccak256 hash.
#[derive(Copy, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Keccak256(pub(crate) [u8; 32]);

impl Keccak256 {
    /// Computes a Keccak256 hash of a message.
    pub fn new(msg: &[u8]) -> Self {
        Self(sha3::Keccak256::new().chain(msg).finalize().into())
    }

    /// Returns a reference to the bytes of this hash.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl ByteFmt for Keccak256 {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        Ok(Self(bytes.try_into()?))
    }

    fn encode(&self) -> Vec<u8> {
        self.0.to_vec()
    }
}
