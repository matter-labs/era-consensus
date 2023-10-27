//! BLS signature scheme for BN254 curve.

use std::collections::BTreeMap;

use ark_bn254::{Bn254, Fr, G1Projective as G1, G2Projective as G2};
use ark_ec::AffineRepr as _;
use ark_ec::bn::Bn;
use ark_ec::Group as _;
use ark_ec::pairing::Pairing as _;
use ark_ec::pairing::PairingOutput;
use num_traits::Zero as _;
use rand::Rng;

pub use crate::bn254_2::error::Error;
use crate::ByteFmt;

mod error;

mod testonly;
#[cfg(test)]
mod tests;
pub mod hash;

pub struct SecretKey(pub Fr);

impl SecretKey {
    /// Generates a random secret key
    pub fn random<R: Rng>(rng: &mut R) -> Self {
        let scalar = Fr::new(rng.gen());
        SecretKey(scalar)
    }

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
        panic!("implement");
        // bn254::PrivateKey::try_from(bytes)
        //     .map(Self)
        //     .map_err(|e| anyhow!("Failed to decode secret key: {e:?}"))
    }

    fn encode(&self) -> Vec<u8> {
        panic!("implement");
        // self.0.to_bytes().unwrap()
    }
}

/// Type safety wrapper around a `bn254` public key.
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
        panic!("implement");
        // bn254::PublicKey::from_compressed(bytes)
        //     .map(Self)
        //     .map_err(|err| anyhow!("Error decoding public key: {err:?}"))
    }

    fn encode(&self) -> Vec<u8> {
        panic!("implement");
        // self.0.to_compressed().unwrap()
    }
}

impl std::hash::Hash for PublicKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        panic!("implement");
        // state.write(&self.0.to_compressed().unwrap())
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

#[derive(Clone, Debug)]
pub struct Signature(pub G1);

impl Signature {
    pub fn verify(&self, msg: &[u8], pk: &PublicKey) -> Result<(), Error> {
        let hash_point = hash::hash_to_g1(msg);

        // First pair: e(H(m): G1, pk: G2)
        let first = Bn254::pairing(hash_point, pk.0);
        // Second pair: e(sig: G1, generator: G2)
        let second = Bn254::pairing(self.0, G2::generator());

        if first == second {
            Ok(())
        } else {
            Err(Error::SignatureVerificationFailure)
        }
    }
}

fn multi_pairing(pairs: &[(G1, G2)]) -> PairingOutput<Bn254> {
    let mut g1: Vec<G1> = Vec::new();
    let mut g2: Vec<G2> = Vec::new();
    for (p, q) in pairs {
        g1.push(p.clone());
        g2.push(q.clone());
    }

    Bn254::multi_pairing(g1, g2)
}

impl PartialEq for Signature {
    fn eq(&self, other: &Self) -> bool {
        ByteFmt::encode(self).eq(&ByteFmt::encode(other))
    }
}

impl Eq for Signature {}

impl ByteFmt for Signature {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        panic!("implement");
        // bn254::Signature::from_compressed(bytes)
        //     .map(Self)
        //     .map_err(|err| anyhow!("Error decoding signature: {err:?}"))
    }
    fn encode(&self) -> Vec<u8> {
        panic!()
        // self.0.to_compressed().unwrap()
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AggregateSignature(Signature);

impl AggregateSignature {
    pub fn aggregate<'a>(sigs: impl IntoIterator<Item=&'a Signature>) -> Self {
        panic!("implement");
        // let sigs: Vec<bn254::Signature> = sigs.into_iter().map(|s| s.0).collect();
        // let mut agg = sigs[0];
        // for i in 1..sigs.len() {
        //     agg = agg + sigs[i];
        // }
        //
        // AggregateSignature(Signature(agg))
    }

    pub fn verify<'a>(
        &self,
        msgs_and_pks: impl Iterator<Item=(&'a [u8], &'a PublicKey)>,
    ) -> Result<(), Error> {
        // Aggregate public keys if they are signing the same hash. Each public key aggregated
        // is one fewer pairing to calculate.
        let mut tree_map: BTreeMap<_, PublicKey> = BTreeMap::new();

        for (msg, pk) in msgs_and_pks {
            if let Some(existing_pk) = tree_map.get_mut(msg) {
                let agg = PublicKey(existing_pk.0 + pk.0);
                tree_map.insert(msg, agg);
            } else {
                tree_map.insert(msg, (*pk).clone());
            }
        }

        for (msg, pk) in tree_map {
            if let Err(err) = self.0.verify(msg, &pk) {
                return Err(err);
            }
        }

        Ok(())
    }
}

impl ByteFmt for AggregateSignature {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        let signature = Signature::decode(bytes)?;
        Ok(AggregateSignature(signature))
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
