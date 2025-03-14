//! ECDSA signatures over the Secp256k1 curve, chosen to work with EVM precompiles.

use std::hash::Hash;

use anyhow::bail;
use zeroize::ZeroizeOnDrop;

use crate::{keccak256::Keccak256, ByteFmt};

mod testonly;

#[cfg(test)]
mod tests;

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
        let hash = Keccak256::new(msg);
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
    /// Standard Recover ID.
    ///
    /// To verify signatures with Solidity, for example with the [OpenZeppelin](https://docs.openzeppelin.com/contracts/2.x/api/cryptography#ECDSA-recover-bytes32-bytes-)
    /// library, we need to shift this by 27 when it's serialized to bytes. See [ECDSA.sol](https://github.com/OpenZeppelin/openzeppelin-contracts/blob/de4154710bcc7c6ca5417097f34ce14e9205c3ac/contracts/utils/cryptography/ECDSA.sol#L128-L136).
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
        let hash = Keccak256::new(msg);
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
        anyhow::ensure!(
            expected == recovered,
            "PublicKey mismatch: expected {}, got {}",
            hex::encode(expected.encode()),
            hex::encode(recovered.encode())
        );
        Ok(())
    }
}

impl ByteFmt for Signature {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        anyhow::ensure!(
            bytes.len() == SIGNATURE_LENGTH,
            "unexpected signature length: {}",
            bytes.len()
        );
        let recid = normalize_recovery_id(bytes[64]);
        let Some(recid) = k256::ecdsa::RecoveryId::from_byte(recid) else {
            bail!("unexpected recovery ID: {}", bytes[64]);
        };
        let sig = k256::ecdsa::Signature::from_slice(&bytes[..64])?;
        Ok(Self { sig, recid })
    }

    fn encode(&self) -> Vec<u8> {
        let mut bz = vec![0u8; SIGNATURE_LENGTH];
        let (r, s) = self.sig.split_bytes();
        bz[..32].copy_from_slice(&r);
        bz[32..64].copy_from_slice(&s);
        bz[64] = self.recid.to_byte() + 27;
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

/// Normalize the V in signatures from Ethereum tooling.
///
/// Based on <https://github.com/gakonst/ethers-rs/blob/51fe937f6515689b17a3a83b74a05984ad3a7f11/ethers-core/src/types/signature.rs#L202>
fn normalize_recovery_id(v: u8) -> u8 {
    match v {
        // Case 0: raw/bare
        v @ 0..=26 => v % 4,
        // Case 2: non-eip155 v value
        v @ 27..=34 => (v - 27) % 4,
        // Case 3: eip155 V value
        v @ 35.. => (v - 1) % 2,
    }
}
