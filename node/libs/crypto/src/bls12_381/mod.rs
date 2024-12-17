//! This module implements the BLS signature over the BLS12_381 curve.
//! This is just an adapter of `blst`, exposing zksync-bft-specific API.
//! The implementation is based on the [IRTF draft v5](https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-bls-signature-05).
//!
//! This implementation does NOT protect against rogue key attacks (see https://crypto.stanford.edu/~dabo/pubs/papers/BLSmultisig.html).
//! We expect signers to separately prove knowledge of the secret key, called proof of possession (POP). This library is meant to be used
//! with validators, where each validator registers their public key on-chain together with a POP (a signature over their public key
//! is sufficient).

use crate::ByteFmt;
use blst::{min_sig as bls, BLST_ERROR};
use rand::Rng as _;
use std::{
    collections::BTreeMap,
    fmt::{Debug, Formatter},
};
use zeroize::ZeroizeOnDrop;

#[cfg(test)]
mod tests;

pub mod testonly;

/// The domain separation tag for this signature scheme.
pub const DST: &[u8] = b"MATTER_LABS_CONSENSUS_BLS_SIG_BLS12381G2_XMD:SHA-256_SSWU_RO_POP_";

/// The domain separation tag for the proof of possession.
pub const DST_POP: &[u8] = b"MATTER_LABS_CONSENSUS_BLS_POP_BLS12381G2_XMD:SHA-256_SSWU_RO_";

/// The byte-length of a BLS signature when serialized in compressed form.
pub const SIGNATURE_BYTES_LEN: usize = 48;

/// Represents the signature at infinity.
pub const INFINITY_SIGNATURE: [u8; SIGNATURE_BYTES_LEN] = [
    0xc0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// Type safety wrapper around a `blst` SecretKey
#[derive(ZeroizeOnDrop)]
pub struct SecretKey(bls::SecretKey);

impl SecretKey {
    /// Generates a secret key from a cryptographically-secure entropy source.
    pub fn generate() -> Self {
        // This unwrap is safe as the blst library method will only error if provided less than 32 bytes of key material
        Self(bls::SecretKey::key_gen_v4_5(&rand::rngs::OsRng.gen::<[u8; 32]>(), &[], &[]).unwrap())
    }

    /// Produces a signature using this [`SecretKey`]
    pub fn sign(&self, msg: &[u8]) -> Signature {
        Signature(self.0.sign(msg, DST, &[]))
    }

    /// Produces a proof of possession for the public key corresponding to this [`SecretKey`]
    pub fn sign_pop(&self) -> ProofOfPossession {
        let msg = self.public().encode();
        ProofOfPossession(self.0.sign(&msg, DST_POP, &[]))
    }

    /// Gets the corresponding [`PublicKey`] for this [`SecretKey`]
    #[inline]
    pub fn public(&self) -> PublicKey {
        PublicKey(self.0.sk_to_pk())
    }
}

impl ByteFmt for SecretKey {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        bls::SecretKey::from_bytes(bytes)
            .map(Self)
            .map_err(|e| anyhow::format_err!("Failed to decode secret key: {e:?}"))
    }

    fn encode(&self) -> Vec<u8> {
        self.0.to_bytes().to_vec()
    }
}

impl Debug for SecretKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "SecretKey({:?})", self.public())
    }
}

impl PartialEq for SecretKey {
    fn eq(&self, other: &Self) -> bool {
        self.public() == other.public()
    }
}

/// Type safety wrapper around a `blst` public key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicKey(bls::PublicKey);

impl std::hash::Hash for PublicKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write(&self.0.to_bytes());
    }
}

impl ByteFmt for PublicKey {
    /// This method also checks if the public key is not infinity and if it is in the correct subgroup.
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        bls::PublicKey::key_validate(bytes)
            .map(Self)
            .map_err(|e| anyhow::format_err!("Failed to decode public key: {e:?}"))
    }

    fn encode(&self) -> Vec<u8> {
        self.0.to_bytes().to_vec()
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
    pub fn verify(&self, msg: &[u8], pk: &PublicKey) -> anyhow::Result<()> {
        let result = self.0.verify(true, msg, DST, &[], &pk.0, true);

        match result {
            BLST_ERROR::BLST_SUCCESS => Ok(()),
            err => Err(anyhow::format_err!(
                "Signature verification failure: {err:?}"
            )),
        }
    }
}

impl ByteFmt for Signature {
    /// This method also checks if the signature is in the correct subgroup.
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        bls::Signature::sig_validate(bytes, false)
            .map(Self)
            .map_err(|err| anyhow::format_err!("Error decoding signature: {err:?}"))
    }

    fn encode(&self) -> Vec<u8> {
        self.0.to_bytes().to_vec()
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

impl std::hash::Hash for Signature {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        ByteFmt::encode(self).hash(state)
    }
}

/// Type safety wrapper around a `blst` aggregate signature
/// WARNING: any change to this struct may invalidate preexisting signatures. See `TimeoutQC` docs.
#[derive(Clone, Debug)]
pub struct AggregateSignature(bls::AggregateSignature);

impl Default for AggregateSignature {
    fn default() -> Self {
        // This can't fail in production since we are decoding a known value.
        Self::decode(&INFINITY_SIGNATURE).unwrap()
    }
}

impl AggregateSignature {
    /// Add a signature to the aggregation.
    pub fn add(&mut self, sig: &Signature) {
        // This cannot fail since we are not validating the signature.
        self.0.add_signature(&sig.0, false).unwrap()
    }

    /// Generate a new aggregate signature from a list of signatures.
    pub fn aggregate<'a>(sigs: impl IntoIterator<Item = &'a Signature>) -> Self {
        let mut agg = Self::default();
        for sig in sigs {
            agg.add(sig);
        }
        agg
    }

    /// Verifies an aggregated signature for multiple messages against the provided list of public keys.
    /// This method expects one public key per message, otherwise it will fail. Note however that
    /// if there are any duplicate messages, the public keys will be aggregated before verification.
    pub fn verify<'a>(
        &self,
        msgs_and_pks: impl Iterator<Item = (&'a [u8], &'a PublicKey)>,
    ) -> anyhow::Result<()> {
        // Aggregate public keys if they are signing the same hash. Each public key aggregated
        // is one fewer pairing to calculate.
        let mut tree_map: BTreeMap<_, bls::AggregatePublicKey> = BTreeMap::new();

        for (msg, pk) in msgs_and_pks {
            if let Some(existing_pk) = tree_map.get_mut(msg) {
                if let Err(err) = existing_pk.add_public_key(&pk.0, false) {
                    return Err(anyhow::format_err!(
                        "Error aggregating public keys: {err:?}"
                    ));
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
        // Due to the `blst` aggregated signatures not having a verify method, this is first converted to a bare signature.
        let result =
            self.0
                .to_signature()
                .aggregate_verify(true, &messages, DST, &public_keys, true);

        match result {
            BLST_ERROR::BLST_SUCCESS => Ok(()),
            err => Err(anyhow::format_err!(
                "Aggregate signature verification failure: {err:?}"
            )),
        }
    }
}

impl ByteFmt for AggregateSignature {
    /// This method also checks if the signature is in the correct subgroup.
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        let sig = bls::Signature::sig_validate(bytes, false)
            .map_err(|err| anyhow::format_err!("Error decoding signature: {err:?}"))?;

        Ok(AggregateSignature(bls::AggregateSignature::from_signature(
            &sig,
        )))
    }

    fn encode(&self) -> Vec<u8> {
        self.0.to_signature().to_bytes().to_vec()
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

impl Eq for AggregateSignature {}

impl PartialEq for AggregateSignature {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_signature() == other.0.to_signature()
    }
}

/// Type safety wrapper around a `blst` proof of possession.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofOfPossession(bls::Signature);

impl ProofOfPossession {
    /// Verifies a proof of possession against the provided public key
    pub fn verify(&self, pk: &PublicKey) -> anyhow::Result<()> {
        let msg = pk.encode();

        let result = self.0.verify(true, &msg, DST_POP, &[], &pk.0, true);

        match result {
            BLST_ERROR::BLST_SUCCESS => Ok(()),
            err => Err(anyhow::format_err!(
                "Proof of possession verification failure: {err:?}"
            )),
        }
    }
}

impl ByteFmt for ProofOfPossession {
    /// This method also checks if the signature is in the correct subgroup.
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        bls::Signature::sig_validate(bytes, false)
            .map(Self)
            .map_err(|err| anyhow::format_err!("Error decoding proof of possession: {err:?}"))
    }

    fn encode(&self) -> Vec<u8> {
        self.0.to_bytes().to_vec()
    }
}

impl PartialOrd for ProofOfPossession {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ProofOfPossession {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        ByteFmt::encode(self).cmp(&ByteFmt::encode(other))
    }
}
