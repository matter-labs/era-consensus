//! BLS signature scheme for the BN254 curve.
//!
//! Disclaimer: the implementation of the pairing-friendly elliptic curve does not run in constant time,
//! hence it does not protect the secret key from side-channel attacks.

use crate::ByteFmt;
use anyhow::Context as _;
use ark_bn254::{Bn254, Fr, G1Projective as G1, G2Projective as G2};
use ark_ec::{
    pairing::{Pairing as _, PairingOutput},
    Group as _,
};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
pub use error::Error;
use num_traits::Zero as _;
use std::collections::HashMap;

#[doc(hidden)]
pub mod error;

#[cfg(test)]
mod tests;

pub mod hash;
mod testonly;

/// Type safety wrapper around a scalar value.
pub struct SecretKey(Fr);

impl SecretKey {
    /// Gets the corresponding [`PublicKey`] for this [`SecretKey`]
    pub fn public(&self) -> PublicKey {
        let p = G2::generator() * self.0;
        PublicKey(p)
    }

    /// Produces a signature using this [`SecretKey`]
    pub fn sign(&self, msg: &[u8]) -> Signature {
        let hash_point = hash::hash_to_g1(msg);
        let sig = hash_point * self.0;
        Signature(sig)
    }
}

impl ByteFmt for SecretKey {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        Fr::deserialize_compressed(bytes)
            .map(Self)
            .context("failed to decode secret key")
    }

    fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.0.serialize_compressed(&mut buf).unwrap();
        buf
    }
}

/// Type safety wrapper around G2.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PublicKey(G2);

impl ByteFmt for PublicKey {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        G2::deserialize_compressed(bytes)
            .map(Self)
            .context("failed to decode public key")
    }

    fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.0.serialize_compressed(&mut buf).unwrap();
        buf
    }
}

impl PartialOrd for PublicKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PublicKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        ByteFmt::encode(self).cmp(&ByteFmt::encode(other))
    }
}

/// Type safety wrapper around a G1 value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature(G1);

impl Signature {
    /// Verifies a signature against the provided public key.
    ///
    /// This function is intentionally non-generic and disallow inlining to ensure that compilation optimizations can be effectively applied.
    /// This optimization is needed for ensuring that tests can run within a reasonable time frame.
    #[inline(never)]
    pub fn verify(&self, msg: &[u8], pk: &PublicKey) -> Result<(), Error> {
        let hash_point = hash::hash_to_g1(msg);

        // First pair: e(H(m): G1, pk: G2)
        let a = Bn254::pairing(hash_point, pk.0);
        // Second pair: e(sig: G1, generator: G2)
        let b = Bn254::pairing(self.0, G2::generator());

        if a == b {
            Ok(())
        } else {
            Err(Error::SignatureVerificationFailure)
        }
    }
}

impl ByteFmt for Signature {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        G1::deserialize_compressed(bytes)
            .map(Self)
            .context("failed to decode signature")
    }
    fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.0.serialize_compressed(&mut buf).unwrap();
        buf
    }
}

impl PartialOrd for Signature {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Signature {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        ByteFmt::encode(self).cmp(&ByteFmt::encode(other))
    }
}
/// Type safety wrapper around [Signature] indicating that it is an aggregated signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AggregateSignature(G1);

impl AggregateSignature {
    /// Generates an aggregate signature from a list of signatures.
    pub fn aggregate<'a>(sigs: impl IntoIterator<Item = &'a Signature>) -> Self {
        let mut agg = G1::zero();
        for sig in sigs {
            agg += sig.0
        }

        AggregateSignature(agg)
    }

    /// This function is intentionally non-generic and disallow inlining to ensure that compilation optimizations can be effectively applied.
    /// This optimization is needed for ensuring that tests can run within a reasonable time frame.
    #[inline(never)]
    fn verify_raw(&self, msgs_and_pks: &[(&[u8], &PublicKey)]) -> Result<(), Error> {
        // Aggregate public keys if they are signing the same hash. Each public key aggregated
        // is one fewer pairing to calculate.
        let mut pairs: HashMap<&[u8], G2> = HashMap::new();
        for (msg, pk) in msgs_and_pks {
            *pairs.entry(msg).or_default() += pk.0;
        }
        // First pair: e(sig: G1, generator: G2)
        let a = Bn254::pairing(self.0, G2::generator());

        // Second pair: e(H(m1): G1, pk1: G2) * ... * e(H(m1000): G1, pk1000: G2)
        let mut b = PairingOutput::zero();
        for (msg, pk) in pairs {
            b += Bn254::pairing(hash::hash_to_g1(msg), pk);
        }

        if a == b {
            Ok(())
        } else {
            Err(Error::AggregateSignatureVerificationFailure)
        }
    }

    /// Verifies an aggregated signature for multiple messages against the provided list of public keys.
    /// This method expects one public key per message, otherwise it will fail. Note however that
    /// If there are any duplicate messages, the public keys will be aggregated before verification.
    pub fn verify<'a>(
        &self,
        msgs_and_pks: impl Iterator<Item = (&'a [u8], &'a PublicKey)>,
    ) -> Result<(), Error> {
        self.verify_raw(&msgs_and_pks.collect::<Vec<_>>()[..])
    }
}

impl ByteFmt for AggregateSignature {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        G1::deserialize_compressed(bytes)
            .map(Self)
            .context("failed to decode aggregate signature")
    }

    fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.0.serialize_compressed(&mut buf).unwrap();
        buf
    }
}

impl PartialOrd for AggregateSignature {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for AggregateSignature {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        ByteFmt::encode(self).cmp(&ByteFmt::encode(other))
    }
}
