use std::{collections::BTreeMap, fmt};

use zksync_consensus_crypto::{secp256k1, ByteFmt, Text, TextFmt};

use super::PublicKey;
use crate::attester::{Msg, MsgHash};

/// A signature of an L1 batch from an attester.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Signature(pub(crate) secp256k1::Signature);

impl Signature {
    /// Verify a message against a public key.
    pub fn verify_msg(&self, msg: &Msg, pk: &PublicKey) -> anyhow::Result<()> {
        self.verify_hash(&msg.hash(), pk)
    }

    /// Verify a message hash against a public key.
    pub fn verify_hash(&self, msg_hash: &MsgHash, pk: &PublicKey) -> anyhow::Result<()> {
        self.0.verify_hash(&ByteFmt::encode(msg_hash), &pk.0)
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
            "attester:signature:secp256k1:{}",
            hex::encode(ByteFmt::encode(&self.0))
        )
    }
    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("attester:signature:secp256k1:")?
            .decode_hex()
            .map(Self)
    }
}

impl fmt::Debug for Signature {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.write_str(&TextFmt::encode(self))
    }
}

/// A collection of signatures from attesters.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct MultiSig(pub(crate) BTreeMap<PublicKey, Signature>);

impl MultiSig {
    /// Check whether an attester has added their signature.
    pub fn contains(&self, key: &PublicKey) -> bool {
        self.0.contains_key(key)
    }

    /// Add an attester signature.
    pub fn add(&mut self, key: PublicKey, sig: Signature) {
        self.0.insert(key, sig);
    }

    /// Iterate over the signer keys.
    pub fn keys(&self) -> impl Iterator<Item = &PublicKey> {
        self.0.keys()
    }

    /// Iterate over the signer keys.
    pub fn iter(&self) -> impl Iterator<Item = (&PublicKey, &Signature)> {
        self.0.iter()
    }

    /// Verify a message against all signatures.
    pub fn verify_msg(&self, msg: &Msg) -> anyhow::Result<()> {
        self.verify_hash(&msg.hash())
    }

    /// Verify a message hash against all signatures.
    pub fn verify_hash(&self, msg_hash: &MsgHash) -> anyhow::Result<()> {
        for (pk, sig) in &self.0 {
            sig.verify_hash(msg_hash, pk)?;
        }
        Ok(())
    }
}
