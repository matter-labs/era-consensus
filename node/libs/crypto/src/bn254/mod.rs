//! This module implements the BLS signature over the BN254 curve.
//! The implementation is based on the [IRTF draft v5](https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-bls-signature-05).
//!
//! Disclaimer: the implementation of the pairing-friendly elliptic curve does not run in constant time,
//! hence it does not protect the secret key from side-channel attacks.
//!

use crate::ByteFmt;
use ff_ce::Field as _;
use pairing::{
    bn256::{Bn256, Fq12, Fr, FrRepr, G1Affine, G1Compressed, G2Affine, G2Compressed, G1, G2},
    ff::{PrimeField, PrimeFieldRepr},
    CurveAffine as _, CurveProjective as _, EncodedPoint as _, Engine as _,
};
use std::{
    collections::HashMap,
    fmt::{Debug, Formatter},
    hash::{Hash, Hasher},
    io::Cursor,
};

#[cfg(test)]
mod tests;

pub mod hash;
mod testonly;

/// Type safety wrapper around a scalar value.
pub struct SecretKey(Fr);

impl SecretKey {
    /// Generates a secret key from a cryptographically-secure entropy source.
    pub fn generate() -> Self {
        loop {
            let fr: Fr = rand04::Rand::rand(&mut rand04::OsRng::new().unwrap());

            if !fr.is_zero() {
                return Self(fr);
            }
        }
    }

    /// Gets the corresponding [`PublicKey`] for this [`SecretKey`]
    pub fn public(&self) -> PublicKey {
        let p = G2Affine::one().mul(self.0);
        let pk = PublicKey(p);

        // Verify public key is valid. Since we already check the validity of a
        // secret key when constructing it, this should never fail (in theory).
        pk.verify().unwrap();

        pk
    }

    /// Produces a signature using this [`SecretKey`]
    pub fn sign(&self, msg: &[u8]) -> Signature {
        let (msg_point, _) = hash::hash_to_point(msg);
        let sig = msg_point.mul(self.0);
        Signature(sig)
    }
}

impl ByteFmt for SecretKey {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        let sk: [u8; 32] = bytes.try_into()?;
        let mut fr_repr = FrRepr::default();
        fr_repr.read_be(Cursor::new(sk))?;
        let fr = Fr::from_repr(fr_repr)?;

        anyhow::ensure!(!fr.is_zero(), "Secret key can't be zero");

        Ok(SecretKey(fr))
    }

    fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.0.into_repr().write_be(&mut buf).unwrap();
        buf
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

/// Type safety wrapper around G2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicKey(G2);

impl PublicKey {
    /// Checks if the public key is not the identity element and is in the correct subgroup. Verifying signatures
    /// against public keys that are not valid is insecure.
    fn verify(&self) -> anyhow::Result<()> {
        // Check that the point is not the identity element.
        anyhow::ensure!(!self.0.is_zero(), "Public key can't be zero");

        // We multiply the point by the order and check if the result is the identity element.
        // If it is, then the point is on the correct subgroup.
        let order = Fr::char();
        let mut p = self.0;
        p.mul_assign(order);
        anyhow::ensure!(p.is_zero(), "Public key must be in the correct subgroup");

        Ok(())
    }
}

impl Hash for PublicKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write(&self.encode());
    }
}

impl ByteFmt for PublicKey {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        let arr: [u8; 64] = bytes.try_into()?;
        let p = G2Compressed::from_fixed_bytes(arr).into_affine()?;
        let pk = PublicKey(p.into());

        pk.verify()?;

        Ok(pk)
    }

    fn encode(&self) -> Vec<u8> {
        self.0.into_affine().into_compressed().as_ref().to_vec()
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
    ///
    /// Subgroup checks for signatures are unnecessary when using the G1 group because it has a cofactor of 1,
    /// ensuring all signatures are in the correct subgroup.
    /// Ref: https://hackmd.io/@jpw/bn254#Subgroup-check-for-mathbb-G_1.
    #[inline(never)]
    pub fn verify(&self, msg: &[u8], pk: &PublicKey) -> anyhow::Result<()> {
        // Verify public key is valid. Since we already check the validity of a
        // public key when constructing it, this should never fail (in theory).
        pk.verify().unwrap();

        let (msg_point, _) = hash::hash_to_point(msg);

        // First pair: e(H(m): G1, pk: G2)
        let a = Bn256::pairing(msg_point, pk.0);
        // Second pair: e(sig: G1, generator: G2)
        let b = Bn256::pairing(self.0, G2Affine::one());

        anyhow::ensure!(a == b, "Signature verification failure");

        Ok(())
    }
}

impl ByteFmt for Signature {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        let arr: [u8; 32] = bytes.try_into()?;
        let p = G1Compressed::from_fixed_bytes(arr)
            .into_affine()?
            .into_projective();
        Ok(Signature(p))
    }

    fn encode(&self) -> Vec<u8> {
        self.0.into_affine().into_compressed().as_ref().to_vec()
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

impl Default for AggregateSignature {
    fn default() -> Self {
        Self(G1Affine::zero().into_projective())
    }
}

impl AggregateSignature {
    /// Add a signature to the aggregation.
    pub fn add(&mut self, sig: &Signature) {
        self.0.add_assign(&sig.0)
    }

    /// This function is intentionally non-generic and disallow inlining to ensure that compilation optimizations can be effectively applied.
    /// This optimization is needed for ensuring that tests can run within a reasonable time frame.
    #[inline(never)]
    fn verify_raw(&self, msgs_and_pks: &[(&[u8], &PublicKey)]) -> anyhow::Result<()> {
        // Aggregate public keys if they are signing the same hash. Each public key aggregated
        // is one fewer pairing to calculate.
        let mut pairs: HashMap<&[u8], PublicKey> = HashMap::new();

        for (msg, pk) in msgs_and_pks {
            // Verify public key is valid. Since we already check the validity of a
            // public key when constructing it, this should never fail (in theory).
            pk.verify().unwrap();

            pairs
                .entry(msg)
                .and_modify(|agg_pk| agg_pk.0.add_assign(&pk.0))
                .or_insert((*pk).clone());
        }

        // First pair: e(sig: G1, generator: G2)
        let a = Bn256::pairing(self.0, G2::one());

        // Second pair: e(H(m1): G1, pk1: G2) * ... * e(H(m1000): G1, pk1000: G2)
        let mut b = Fq12::one();
        for (msg, pk) in pairs {
            let (msg_point, _) = hash::hash_to_point(msg);
            b.mul_assign(&Bn256::pairing(msg_point, pk.0))
        }

        anyhow::ensure!(a == b, "Aggregate signature verification failure");

        Ok(())
    }

    /// Verifies an aggregated signature for multiple messages against the provided list of public keys.
    /// This method expects one public key per message, otherwise it will fail. Note however that
    /// If there are any duplicate messages, the public keys will be aggregated before verification.
    pub fn verify<'a>(
        &self,
        msgs_and_pks: impl Iterator<Item = (&'a [u8], &'a PublicKey)>,
    ) -> anyhow::Result<()> {
        self.verify_raw(&msgs_and_pks.collect::<Vec<_>>()[..])
    }
}

impl ByteFmt for AggregateSignature {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        let arr: [u8; 32] = bytes.try_into()?;
        let p = G1Compressed::from_fixed_bytes(arr)
            .into_affine()?
            .into_projective();
        Ok(AggregateSignature(p))
    }

    fn encode(&self) -> Vec<u8> {
        self.0.into_affine().into_compressed().as_ref().to_vec()
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
