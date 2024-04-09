//! This module implements the BLS signature over the BLS12_381 curve.
//! This is just an adapter of `blst`, exposing zksync-bft-specific API.
//! The implementation is based on the [IRTF draft v5](https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-bls-signature-05).

use crate::ByteFmt;
use anyhow::{anyhow, bail};
use blst::{min_pk as bls, BLST_ERROR};
use std::collections::BTreeMap;

#[cfg(test)]
mod tests;

pub mod testonly;

/// The byte-length of a BLS public key when serialized in compressed form.
pub const PUBLIC_KEY_BYTES_LEN: usize = 48;

/// Represents the public key at infinity.
pub const INFINITY_PUBLIC_KEY: [u8; PUBLIC_KEY_BYTES_LEN] = [
    0xc0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// Type safety wrapper around a `blst` SecretKey
pub struct SecretKey(bls::SecretKey);

impl SecretKey {
    /// Generates a secret key from provided key material
    pub fn generate(key_material: [u8; 32]) -> Self {
        // This unwrap is safe as the blst library method will only error if provided less than 32 bytes of key material
        Self(bls::SecretKey::key_gen_v4_5(&key_material, &[], &[]).unwrap())
    }

    /// Produces a signature using this [`SecretKey`]
    pub fn sign(&self, msg: &[u8]) -> Signature {
        let signature = self.0.sign(msg, &[], &[]);
        Signature(signature)
    }

    /// Gets the corresponding [`PublicKey`] for this [`SecretKey`]
    #[inline]
    pub fn public(&self) -> PublicKey {
        let public_key = self.0.sk_to_pk();
        PublicKey(public_key)
    }
}

impl ByteFmt for SecretKey {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        bls::SecretKey::from_bytes(bytes)
            .map(Self)
            .map_err(|e| anyhow!("Failed to decode secret key: {e:?}"))
    }

    fn encode(&self) -> Vec<u8> {
        self.0.to_bytes().to_vec()
    }
}

/// Type safety wrapper around a `blst` public key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicKey(bls::PublicKey);

impl ByteFmt for PublicKey {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        if bytes == INFINITY_PUBLIC_KEY {
            bail!(Error::InvalidInfinityPublicKey)
        }
        bls::PublicKey::from_bytes(bytes)
            .map(Self)
            .map_err(|err| anyhow!("Error decoding public key: {err:?}"))
    }

    fn encode(&self) -> Vec<u8> {
        self.0.to_bytes().to_vec()
    }
}

impl std::hash::Hash for PublicKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write(&self.0.to_bytes());
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

/// Type safety wrapper around a `blst` signature
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature(bls::Signature);

impl Signature {
    /// Verifies a signature against the provided public key
    pub fn verify(&self, msg: &[u8], pk: &PublicKey) -> Result<(), Error> {
        let result = self.0.verify(true, msg, &[], &[], &pk.0, true);

        match result {
            BLST_ERROR::BLST_SUCCESS => Ok(()),
            err => Err(Error::SignatureVerification(err)),
        }
    }
}

impl ByteFmt for Signature {
    fn encode(&self) -> Vec<u8> {
        self.0.to_bytes().to_vec()
    }
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        bls::Signature::from_bytes(bytes)
            .map(Self)
            .map_err(|err| anyhow!("Error decoding signature: {err:?}"))
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

/// Type safety wrapper around a `blst` signature indicating that it is an aggregated signature
///
/// Due to the `blst` aggregated signatures not having a verify method, this is stored converted to
/// a bare signature internally.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AggregateSignature(bls::Signature);

impl AggregateSignature {
    /// Generates an aggregate signature from a list of signatures
    pub fn aggregate<'a>(sigs: impl IntoIterator<Item = &'a Signature>) -> Result<Self, Error> {
        let sigs: Vec<&bls::Signature> = sigs.into_iter().map(|s| &s.0).collect();

        let aggregate = bls::AggregateSignature::aggregate(&sigs[..], true)
            .map_err(Error::SignatureAggregation)?;

        Ok(AggregateSignature(aggregate.to_signature()))
    }

    /// Verifies an aggregated signature for multiple messages against the provided list of public keys.
    /// This method expects one public key per message, otherwise it will fail. Note however that
    /// If there are any duplicate messages, the public keys will be aggregated before verification.
    pub fn verify<'a>(
        &self,
        msgs_and_pks: impl Iterator<Item = (&'a [u8], &'a PublicKey)>,
    ) -> Result<(), Error> {
        // Aggregate public keys if they are signing the same hash. Each public key aggregated
        // is one fewer pairing to calculate.
        let mut tree_map: BTreeMap<_, bls::AggregatePublicKey> = BTreeMap::new();

        for (msg, pk) in msgs_and_pks {
            if let Some(existing_pk) = tree_map.get_mut(msg) {
                if let Err(err) = existing_pk.add_public_key(&pk.0, false) {
                    return Err(Error::AggregateSignatureVerification(err));
                }
            } else {
                tree_map.insert(msg, bls::AggregatePublicKey::from_public_key(&pk.0));
            }
        }

        let (messages, public_keys): (Vec<_>, Vec<_>) = tree_map
            .iter()
            .map(|(msg, agg_pk)| (msg, agg_pk.to_public_key()))
            .unzip();

        let public_keys: Vec<&bls::PublicKey> = public_keys.iter().collect();

        // Verify the signature.
        let result = self
            .0
            .aggregate_verify(true, &messages, &[], &public_keys, true);

        match result {
            BLST_ERROR::BLST_SUCCESS => Ok(()),
            err => Err(Error::AggregateSignatureVerification(err)),
        }
    }
}

impl ByteFmt for AggregateSignature {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        let signature = bls::Signature::from_bytes(bytes)
            .map_err(|err| anyhow!("Error decoding signature: {err:?}"))?;
        Ok(AggregateSignature(signature))
    }

    fn encode(&self) -> Vec<u8> {
        self.0.to_bytes().to_vec()
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

/// Error type for generating and interacting with BLS keys/signatures
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Signature verification failure
    #[error("Signature verification failure: {0:?}")]
    SignatureVerification(BLST_ERROR),
    /// Aggregate signature verification failure
    #[error("Aggregate signature verification failure: {0:?}")]
    AggregateSignatureVerification(BLST_ERROR),
    /// Error aggregating signatures
    #[error("Error aggregating signatures: {0:?}")]
    SignatureAggregation(BLST_ERROR),
    /// Infinity public key.
    #[error("Error infinity public key")]
    InvalidInfinityPublicKey,
}
