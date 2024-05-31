//! ECDSA signatures over the Secp256k1 curve, chosen to work with EVM precompiles.

use std::hash::Hash;

use anyhow::bail;
use zeroize::ZeroizeOnDrop;

use crate::{keccak256, ByteFmt};

mod testonly;

const SIGNATURE_LENGTH: usize = 65;

/// Secp256k1 secret key
#[derive(ZeroizeOnDrop, PartialEq, Eq)]
pub struct SecretKey(k256::ecdsa::SigningKey);

impl SecretKey {
    /// Generates a secret key from a cryptographically-secure entropy source.
    pub fn generate() -> Self {
        Self(k256::SecretKey::random(&mut rand::rngs::OsRng).into())
    }

    /// Gets the corresponding [`PublicKey`] for this [`SecretKey`]
    pub fn public(&self) -> PublicKey {
        PublicKey(*self.0.verifying_key())
    }

    /// Hashes the message with Keccak256 and signs it.
    pub fn sign(&self, msg: &[u8]) -> anyhow::Result<Signature> {
        let hash = keccak256::Keccak256::new(msg);
        self.sign_hash(hash.as_bytes())
    }

    /// Signs a message digest.
    pub fn sign_hash(&self, hash: &[u8]) -> anyhow::Result<Signature> {
        let (sig, recid) = self.0.sign_prehash_recoverable(hash)?;
        Ok(Signature { sig, recid })
    }
}

impl ByteFmt for SecretKey {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        let sk = k256::ecdsa::SigningKey::from_slice(bytes)?;
        Ok(Self(sk))
    }

    fn encode(&self) -> Vec<u8> {
        self.0.to_bytes().to_vec()
    }
}

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SecretKey({:?})", self.public())
    }
}

/// Secp256k1 public key
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PublicKey(k256::ecdsa::VerifyingKey);

impl ByteFmt for PublicKey {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        let vk = k256::ecdsa::VerifyingKey::from_sec1_bytes(bytes)?;
        Ok(Self(vk))
    }

    fn encode(&self) -> Vec<u8> {
        self.0.to_sec1_bytes().to_vec()
    }
}

impl Hash for PublicKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write(&self.encode())
    }
}

/// Secp256k1 signature
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    sig: k256::ecdsa::Signature,
    // TODO: Look up where we need to shift the recovery ID for Solidity
    recid: k256::ecdsa::RecoveryId,
}

impl Signature {
    /// Verifies a signature against a provided public key, taking the Keccak256 hash of the message.
    pub fn verify(&self, msg: &[u8], pk: &PublicKey) -> anyhow::Result<()> {
        let rec = self.recover(msg)?;
        Self::verify_pk(pk, &rec)
    }

    /// Verifies a signature against a provided public key.
    ///
    /// Expects the input to be a hash of the message.
    pub fn verify_hash(&self, hash: &[u8], pk: &PublicKey) -> anyhow::Result<()> {
        let rec = self.recover_hash(hash)?;
        Self::verify_pk(pk, &rec)
    }

    /// Recovers the public key from the signature, taking the Keccak256 hash of the message.
    pub fn recover(&self, msg: &[u8]) -> anyhow::Result<PublicKey> {
        let hash = keccak256::Keccak256::new(msg);
        self.recover_hash(hash.as_bytes())
    }

    /// Recovers the public key from the signature.
    ///
    /// Expects the input to be a hash of the message.
    pub fn recover_hash(&self, hash: &[u8]) -> anyhow::Result<PublicKey> {
        let vk = k256::ecdsa::VerifyingKey::recover_from_prehash(hash, &self.sig, self.recid)?;
        Ok(PublicKey(vk))
    }

    /// Ensure the recovered public key is the expected one.
    fn verify_pk(expected: &PublicKey, recovered: &PublicKey) -> anyhow::Result<()> {
        anyhow::ensure!(expected == recovered, "PublicKey mismatch");
        Ok(())
    }
}

impl ByteFmt for Signature {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        anyhow::ensure!(
            bytes.len() == SIGNATURE_LENGTH,
            "unexpected signature length",
        );
        let Some(recid) = k256::ecdsa::RecoveryId::from_byte(bytes[64]) else {
            bail!("unexpected recovery ID");
        };
        let sig = k256::ecdsa::Signature::from_slice(&bytes[..64])?;
        Ok(Self { sig, recid })
    }

    fn encode(&self) -> Vec<u8> {
        let mut bz = vec![0u8; SIGNATURE_LENGTH];
        let (r, s) = self.sig.split_bytes();
        bz[..32].copy_from_slice(&r);
        bz[32..64].copy_from_slice(&s);
        bz[64] = self.recid.to_byte();
        bz
    }
}

impl Hash for Signature {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write(&self.encode())
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

/// Seclp256k1 multi-sig
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct AggregateSignature(Vec<Signature>);

impl AggregateSignature {
    /// Add a signature to the aggregate.
    pub fn add(&mut self, sig: Signature) {
        self.0.push(sig)
    }

    /// Verifies an aggregated signature for multiple messages against the provided list of public keys.
    ///
    /// Verification fails if the total number of messages and signatures do not match.
    ///
    /// The method expects the public keys to appear in the same order as their signatures have been added
    /// to the aggregate. The protocol has to ensure that the data structures are assembled in this way.
    ///
    /// This method was added to mimic the bn254 version, but lost its commutative property.
    pub fn verify_hash<'a>(
        &self,
        hashes_and_pks: impl Iterator<Item = (&'a [u8], &'a PublicKey)>,
    ) -> anyhow::Result<()> {
        let mut cnt = 0;
        for (i, (hash, pk)) in hashes_and_pks.enumerate() {
            let Some(sig) = self.0.get(i) else {
                bail!("not enough signatures in aggregate: expected no more than {i}");
            };
            sig.verify_hash(hash, pk)?;
            cnt += 1;
        }
        if cnt < self.0.len() {
            bail!(
                "not enough messages to verify aggregated signature: expected {}; got {cnt}",
                self.0.len()
            );
        }
        Ok(())
    }
}

impl ByteFmt for AggregateSignature {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        anyhow::ensure!(
            bytes.len() % SIGNATURE_LENGTH == 0,
            "unexpected aggregate signature length"
        );

        let sigs = bytes
            .chunks_exact(SIGNATURE_LENGTH)
            .map(Signature::decode)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self(sigs))
    }

    fn encode(&self) -> Vec<u8> {
        self.0.iter().flat_map(|s| s.encode()).collect()
    }
}
