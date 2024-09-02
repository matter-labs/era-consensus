use super::{ProofOfPossession, PublicKey, Signature};
use crate::validator::messages::{Msg, MsgHash, Signed};
use std::{fmt, sync::Arc};
use zksync_consensus_crypto::{bls12_381, ByteFmt, Text, TextFmt};
use zksync_consensus_utils::enum_util::Variant;

/// A secret key for the validator role.
/// SecretKey is put into an Arc, so that we can clone it,
/// without copying the secret all over the RAM.
#[derive(Clone, PartialEq)]
pub struct SecretKey(pub(crate) Arc<bls12_381::SecretKey>);

impl SecretKey {
    /// Generates a secret key from a cryptographically-secure entropy source.
    pub fn generate() -> Self {
        Self(Arc::new(bls12_381::SecretKey::generate()))
    }

    /// Public key corresponding to this secret key.
    pub fn public(&self) -> PublicKey {
        PublicKey(self.0.public())
    }

    /// Signs a strongly typed message.
    pub fn sign_msg<V: Variant<Msg>>(&self, msg: V) -> Signed<V> {
        let msg = msg.insert();
        Signed {
            sig: self.sign_hash(&msg.hash()),
            key: self.public(),
            msg: V::extract(msg).unwrap(),
        }
    }

    /// Sign a message hash.
    pub fn sign_hash(&self, msg_hash: &MsgHash) -> Signature {
        Signature(self.0.sign(&ByteFmt::encode(msg_hash)))
    }

    /// Sign a proof of possession.
    pub fn sign_pop(&self) -> ProofOfPossession {
        ProofOfPossession(self.0.sign_pop())
    }
}

impl ByteFmt for SecretKey {
    fn encode(&self) -> Vec<u8> {
        ByteFmt::encode(&*self.0)
    }

    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        ByteFmt::decode(bytes).map(Arc::new).map(Self)
    }
}

impl TextFmt for SecretKey {
    fn encode(&self) -> String {
        format!(
            "validator:secret:bls12_381:{}",
            hex::encode(ByteFmt::encode(&*self.0))
        )
    }

    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("validator:secret:bls12_381:")?
            .decode_hex()
            .map(Arc::new)
            .map(Self)
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        // The secret itself should never be logged.
        write!(fmt, "<secret for {}>", TextFmt::encode(&self.public()))
    }
}
