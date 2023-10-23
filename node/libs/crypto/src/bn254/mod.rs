//! BLS signature scheme for BN254 curve.

use std::collections::BTreeMap;

use anyhow::anyhow;
use bn254::{ECDSA, PrivateKey};
use rand::Rng;

use crate::bn254::error::Error;
use crate::ByteFmt;

mod error;

#[cfg(test)]
mod tests;

pub struct SecretKey(PrivateKey);

impl SecretKey {
    /// Generates a random secret key
    pub fn random<R: Rng>(rng: &mut R) -> Self {
        let private_key = PrivateKey::random(rng);
        return Self(private_key);
    }

    /// Produces a signature using this [`SecretKey`]
    pub fn sign(&self, msg: &[u8]) -> Signature {
        let sig = ECDSA::sign(&msg, &self.0).unwrap();
        Signature(sig)
    }

    /// Gets the corresponding [`PublicKey`] for this [`SecretKey`]
    pub fn public(&self) -> PublicKey {
        let pk = bn254::PublicKey::from_private_key(&self.0);
        PublicKey(pk)
    }
}


impl ByteFmt for SecretKey {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        bn254::PrivateKey::try_from(bytes)
            .map(Self)
            .map_err(|e| anyhow!("Failed to decode secret key: {e:?}"))
    }

    fn encode(&self) -> Vec<u8> {
        self.0.to_bytes().unwrap()
    }
}

/// Type safety wrapper around a `bn254` public key.
#[derive(Clone)]
pub struct PublicKey(bn254::PublicKey);

impl PartialEq for PublicKey {
    fn eq(&self, other: &Self) -> bool {
        ByteFmt::encode(self).eq(&ByteFmt::encode(other))
    }
}

impl Eq for PublicKey {}

impl ByteFmt for PublicKey {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        bn254::PublicKey::from_compressed(bytes)
            .map(Self)
            .map_err(|err| anyhow!("Error decoding public key: {err:?}"))
    }

    fn encode(&self) -> Vec<u8> {
        self.0.to_compressed().unwrap()
    }
}

impl std::hash::Hash for PublicKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write(&self.0.to_compressed().unwrap())
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
pub struct Signature(bn254::Signature);

impl Signature {
    pub fn verify(&self, msg: &[u8], pk: &PublicKey) -> Result<(), Error> {
        let result = ECDSA::verify(msg, &self.0, &pk.0);
        match result {
            Ok(()) => Ok(()),
            Err(err) => Err(Error::SignatureVerification(err)),
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
        bn254::Signature::from_compressed(bytes)
            .map(Self)
            .map_err(|err| anyhow!("Error decoding signature: {err:?}"))
    }
    fn encode(&self) -> Vec<u8> {
        self.0.to_compressed().unwrap()   }
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
    pub fn aggregate<'a>(sigs: impl IntoIterator<Item = &'a Signature>) -> Self {
        let sigs: Vec<bn254::Signature> = sigs.into_iter().map(|s| s.0).collect();
        let mut agg = sigs[0];
        for i in 1..sigs.len() {
            agg = agg + sigs[i];
        }

        AggregateSignature(Signature(agg))
    }

    pub fn verify<'a>(
        &self,
        msgs_and_pks: impl Iterator<Item = (&'a [u8], &'a PublicKey)>,
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
