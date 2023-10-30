//! BLS signature scheme for the BN254 curve.

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::hash::Hasher;

use anyhow::anyhow;
use ark_bn254::{Bn254, Fr, G1Projective as G1, G2Projective as G2};
use ark_ec::pairing::Pairing as _;
use ark_ec::pairing::PairingOutput;
use ark_ec::Group as _;
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use num_traits::Zero as _;

pub use error::Error;

use crate::ByteFmt;

#[doc(hidden)]
pub mod error;

#[cfg(test)]
mod tests;

pub mod hash;
mod testonly;

/// Type safety wrapper around a scalar value.
pub struct SecretKey(pub Fr);

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
            .map_err(|e| anyhow!("failed to decode secret key: {e:?}"))
    }

    fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.0.serialize_compressed(&mut buf).unwrap();
        buf
    }
}

/// Type safety wrapper around G2.
#[derive(Clone)]
pub struct PublicKey(pub G2);

impl PartialEq for PublicKey {
    fn eq(&self, other: &Self) -> bool {
        ByteFmt::encode(self).eq(&ByteFmt::encode(other))
    }
}

impl Eq for PublicKey {}

impl ByteFmt for PublicKey {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        G2::deserialize_compressed(bytes)
            .map(Self)
            .map_err(|e| anyhow!("failed to decode public key: {e:?}"))
    }

    fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.0.serialize_compressed(&mut buf).unwrap();
        buf
    }
}

impl std::hash::Hash for PublicKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
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
#[derive(Clone, Debug)]
pub struct Signature(pub G1);

impl Signature {
    /// Verifies a signature against the provided public key
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

impl PartialEq for Signature {
    fn eq(&self, other: &Self) -> bool {
        ByteFmt::encode(self).eq(&ByteFmt::encode(other))
    }
}

impl Eq for Signature {}

impl ByteFmt for Signature {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        G1::deserialize_compressed(bytes)
            .map(Self)
            .map_err(|e| anyhow!("failed to decode signature: {e:?}"))
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
pub struct AggregateSignature(Signature);

impl AggregateSignature {
    /// Generates an aggregate signature from a list of signatures.
    pub fn aggregate<'a>(sigs: impl IntoIterator<Item = &'a Signature>) -> Self {
        let mut agg = G1::zero();
        for sig in sigs {
            agg += sig.0
        }

        AggregateSignature(Signature(agg))
    }

    /// Verifies an aggregated signature for multiple messages against the provided list of public keys.
    /// This method expects one public key per message, otherwise it will fail. Note however that
    /// If there are any duplicate messages, the public keys will be aggregated before verification.
    pub fn verify<'a>(
        &self,
        msgs_and_pks: impl Iterator<Item = (&'a [u8], &'a PublicKey)>,
    ) -> Result<(), Error> {
        let msgs_and_pks = Self::aggregate_pk(msgs_and_pks);

        // First pair: e(sig: G1, generator: G2)
        let a = Bn254::pairing(self.0 .0, G2::generator());

        // Second pair: e(H(m1): G1, pk1: G2) * ... * (H(m1000): G1, pk1000: G2)
        let mut b = PairingOutput::zero();
        for (msg, pk) in msgs_and_pks {
            let hash_point = hash::hash_to_g1(msg);
            b += Bn254::pairing(hash_point, pk.0);
        }

        if a == b {
            Ok(())
        } else {
            Err(Error::AggregateSignatureVerificationFailure)
        }
    }

    fn aggregate_pk<'a>(
        msgs_and_pks: impl Iterator<Item = (&'a [u8], &'a PublicKey)>,
    ) -> impl Iterator<Item = (&'a [u8], PublicKey)> {
        // Aggregate public keys if they are signing the same hash. Each public key aggregated
        // is one fewer pairing to calculate.
        let mut tree_map: BTreeMap<&[u8], PublicKey> = BTreeMap::new();

        for (msg, pk) in msgs_and_pks {
            if let Some(existing_pk) = tree_map.get_mut(msg) {
                existing_pk.0 += pk.0;
            } else {
                tree_map.insert(msg, pk.clone());
            }
        }

        tree_map.into_iter()
    }
}

impl ByteFmt for AggregateSignature {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        let sig = Signature::decode(bytes)?;
        Ok(AggregateSignature(sig))
    }

    fn encode(&self) -> Vec<u8> {
        Signature::encode(&self.0)
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