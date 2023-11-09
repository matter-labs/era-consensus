//! BLS signature scheme for the BN254 curve.
//!
//! Disclaimer: the implementation of the pairing-friendly elliptic curve does not run in constant time,
//! hence it does not protect the secret key from side-channel attacks.

use crate::ByteFmt;
pub use error::Error;
use ff_ce::Field as _;
use pairing::{
    bn256::{Bn256, Fq12, Fr, FrRepr, G1Affine, G1Compressed, G2Affine, G2Compressed, G1, G2},
    ff::{PrimeField, PrimeFieldRepr},
    CurveAffine as _, CurveProjective as _, EncodedPoint as _, Engine as _,
};
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    io::Cursor,
};

#[doc(hidden)]
pub mod error;

#[cfg(test)]
mod tests;

pub mod hash;
mod testonly;

/// Type safety wrapper around a scalar value.
#[derive(Debug, PartialEq)]
pub struct SecretKey(Fr);

impl SecretKey {
    /// Gets the corresponding [`PublicKey`] for this [`SecretKey`]
    pub fn public(&self) -> PublicKey {
        let p = G2Affine::one().mul(self.0);
        PublicKey(p)
    }

    /// Produces a signature using this [`SecretKey`]
    pub fn sign(&self, msg: &[u8]) -> Signature {
        let hash_point = hash::hash_to_g1(msg);
        let sig = hash_point.mul(self.0);
        Signature(sig)
    }
}

impl ByteFmt for SecretKey {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        let mut fr_repr = FrRepr::default();
        fr_repr.read_be(Cursor::new(bytes))?;
        let fr = Fr::from_repr(fr_repr)?;
        Ok(SecretKey(fr))
    }

    fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.0.into_repr().write_be(&mut buf).unwrap();
        buf
    }
}

/// Type safety wrapper around G2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicKey(G2);

impl Default for PublicKey {
    fn default() -> Self {
        PublicKey(G2::zero())
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
        let p = G2Compressed::from_fixed_bytes(arr)
            .into_affine()?
            .into_projective();
        Ok(PublicKey(p))
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
    #[inline(never)]
    pub fn verify(&self, msg: &[u8], pk: &PublicKey) -> Result<(), Error> {
        let hash_point = hash::hash_to_g1(msg);

        // First pair: e(H(m): G1, pk: G2)
        let a = Bn256::pairing(hash_point, pk.0);
        // Second pair: e(sig: G1, generator: G2)
        let b = Bn256::pairing(self.0, G2Affine::one());

        if a == b {
            Ok(())
        } else {
            Err(Error::SignatureVerificationFailure)
        }
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

impl AggregateSignature {
    /// Generates an aggregate signature from a list of signatures.
    pub fn aggregate<'a>(sigs: impl IntoIterator<Item = &'a Signature>) -> Self {
        let mut agg = G1Affine::zero().into_projective();
        for sig in sigs {
            agg.add_assign(&sig.0)
        }

        AggregateSignature(agg)
    }

    /// This function is intentionally non-generic and disallow inlining to ensure that compilation optimizations can be effectively applied.
    /// This optimization is needed for ensuring that tests can run within a reasonable time frame.
    #[inline(never)]
    fn verify_raw(&self, msgs_and_pks: &[(&[u8], &PublicKey)]) -> Result<(), Error> {
        // Aggregate public keys if they are signing the same hash. Each public key aggregated
        // is one fewer pairing to calculate.
        let mut pairs: HashMap<&[u8], PublicKey> = HashMap::new();
        for (msg, pk) in msgs_and_pks {
            pairs.entry(msg).or_default().0.add_assign(&pk.0);
        }
        // First pair: e(sig: G1, generator: G2)
        let a = Bn256::pairing(self.0, G2::one());

        // Second pair: e(H(m1): G1, pk1: G2) * ... * e(H(m1000): G1, pk1000: G2)
        let mut b = Fq12::one();
        for (msg, pk) in pairs {
            b.mul_assign(&Bn256::pairing(hash::hash_to_g1(msg), pk.0))
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
