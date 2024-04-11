use crate::attester::{Msg, MsgHash};

use super::BatchPublicKey;
use std::fmt;
use zksync_consensus_crypto::{bn254, ByteFmt, Text, TextFmt};

/// A signature of an L1 batch from a validator.
#[derive(Clone, PartialEq, Eq)]
pub struct BatchSignature(pub(crate) bn254::Signature);

impl BatchSignature {
    /// Verify a message against a public key.
    pub fn verify_msg(&self, msg: &Msg, pk: &BatchPublicKey) -> anyhow::Result<()> {
        self.verify_hash(&msg.hash(), pk)
    }

    /// Verify a message hash against a public key.
    pub fn verify_hash(&self, msg_hash: &MsgHash, pk: &BatchPublicKey) -> anyhow::Result<()> {
        self.0.verify(&ByteFmt::encode(msg_hash), &pk.0)
    }
}

impl ByteFmt for BatchSignature {
    fn encode(&self) -> Vec<u8> {
        ByteFmt::encode(&self.0)
    }
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        ByteFmt::decode(bytes).map(Self)
    }
}

impl TextFmt for BatchSignature {
    fn encode(&self) -> String {
        format!(
            "validator:batch_signature:bn254:{}",
            hex::encode(ByteFmt::encode(&self.0))
        )
    }
    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("validator:batch_signature:bn254:")?
            .decode_hex()
            .map(Self)
    }
}

impl fmt::Debug for BatchSignature {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.write_str(&TextFmt::encode(self))
    }
}

impl std::hash::Hash for BatchSignature {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        ByteFmt::encode(self).hash(state)
    }
}
