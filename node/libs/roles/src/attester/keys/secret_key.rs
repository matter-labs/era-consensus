use super::{BatchPublicKey, BatchSignature};
use crate::attester::{L1BatchMsg, MsgHash, SignedBatchMsg};
use std::{fmt, sync::Arc};
use zksync_consensus_crypto::{bn254, ByteFmt, Text, TextFmt};
use zksync_consensus_utils::enum_util::Variant;

/// A secret key for the validator role to sign L1 batches.
/// SecretKey is put into an Arc, so that we can clone it,
/// without copying the secret all over the RAM.
#[derive(Clone, PartialEq)]
pub struct SecretKey(pub(crate) Arc<bn254::SecretKey>);

impl SecretKey {
    /// Generates a batch secret key from a cryptographically-secure entropy source.
    pub fn generate() -> Self {
        Self(Arc::new(bn254::SecretKey::generate()))
    }

    /// Batch public key corresponding to this batch secret key.
    pub fn public(&self) -> BatchPublicKey {
        BatchPublicKey(self.0.public())
    }

    /// Signs a strongly typed message.
    pub fn sign_batch_msg(&self, msg: L1BatchMsg) -> SignedBatchMsg {
        let msg = msg.insert();
        SignedBatchMsg {
            sig: self.sign_hash(&msg.hash()),
            key: self.public(),
            msg: L1BatchMsg::extract(msg).unwrap(),
        }
    }

    /// Sign a message hash.
    pub fn sign_hash(&self, msg_hash: &MsgHash) -> BatchSignature {
        BatchSignature(self.0.sign(&ByteFmt::encode(msg_hash)))
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
            "validator:batch_secret:bn254:{}",
            hex::encode(ByteFmt::encode(&*self.0))
        )
    }

    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("validator:batch_secret:bn254:")?
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
