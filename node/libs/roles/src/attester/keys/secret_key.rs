use super::{PublicKey, Signature};
use crate::attester::{Batch, Msg, MsgHash, Signed};
use std::{fmt, sync::Arc};
use zksync_consensus_crypto::{bn254, ByteFmt, Text, TextFmt};
use zksync_consensus_utils::enum_util::Variant;

/// A secret key for the attester role to sign L1 batches.
/// SecretKey is put into an Arc, so that we can clone it,
/// without copying the secret all over the RAM.
#[derive(Clone, PartialEq)]
pub struct SecretKey(pub(crate) Arc<bn254::SecretKey>);

impl SecretKey {
    /// Generates a batch secret key from a cryptographically-secure entropy source.
    pub fn generate() -> Self {
        Self(Arc::new(bn254::SecretKey::generate()))
    }

    /// Public key corresponding to this secret key.
    pub fn public(&self) -> PublicKey {
        PublicKey(self.0.public())
    }

    /// Signs a batch message.
    pub fn sign_msg<V>(&self, msg: Batch) -> Signed<V>
    where
        V: Variant<Msg>,
    {
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
            "attester:secret:bn254:{}",
            hex::encode(ByteFmt::encode(&*self.0))
        )
    }

    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("attester:secret:bn254:")?
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
