use super::{Error, PublicKey};
use crate::validator::messages::{Msg, MsgHash};
use zksync_consensus_crypto::{bn254, ByteFmt, Text, TextFmt};
use std::fmt;

/// A signature from a validator.
#[derive(Clone, PartialEq, Eq)]
pub struct Signature(pub(crate) bn254::Signature);

impl Signature {
    /// Verify a message against a public key.
    pub fn verify_msg(&self, msg: &Msg, pk: &PublicKey) -> Result<(), Error> {
        self.verify_hash(&msg.hash(), pk)
    }

    /// Verify a message hash against a public key.
    pub fn verify_hash(&self, msg_hash: &MsgHash, pk: &PublicKey) -> Result<(), Error> {
        self.0.verify(&ByteFmt::encode(msg_hash), &pk.0)
    }
}

impl ByteFmt for Signature {
    fn encode(&self) -> Vec<u8> {
        ByteFmt::encode(&self.0)
    }
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        ByteFmt::decode(bytes).map(Self)
    }
}

impl TextFmt for Signature {
    fn encode(&self) -> String {
        format!(
            "validator:signature:bn254:{}",
            hex::encode(ByteFmt::encode(&self.0))
        )
    }
    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("validator:signature:bn254:")?
            .decode_hex()
            .map(Self)
    }
}

impl fmt::Debug for Signature {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.write_str(&TextFmt::encode(self))
    }
}

impl std::hash::Hash for Signature {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        ByteFmt::encode(self).hash(state)
    }
}
