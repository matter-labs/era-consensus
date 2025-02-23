//! Wrappers for the Keccak256 cryptographic hash algorithm.
use sha3::{digest::Update as _, Digest as _};

use crate::ByteFmt;

#[cfg(test)]
mod test;
pub mod testonly;

/// Keccak256 hash.
/// WARNING: any change to this struct may invalidate preexisting signatures. See `TimeoutQC` docs.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Keccak256(pub(crate) [u8; 32]);

impl Keccak256 {
    /// Computes a Keccak256 hash of a message.
    pub fn new(msg: &[u8]) -> Self {
        Self(sha3::Keccak256::new().chain(msg).finalize().into())
    }

    /// Interprets the specified `bytes` as a hash digest (i.e., a reverse operation to [`Self::as_bytes()`]).
    /// It is caller's responsibility to ensure that `bytes` are actually a Keccak256 hash digest.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
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
