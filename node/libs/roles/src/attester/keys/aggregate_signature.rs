use super::{PublicKey, Signature};
use crate::attester::{Batch, MsgHash};
use std::fmt;
use zksync_consensus_crypto::{secp256k1, ByteFmt, Text, TextFmt};
use zksync_consensus_utils::enum_util::Variant;

/// An aggregate signature from an attester.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct AggregateSignature(pub(crate) secp256k1::AggregateSignature);

impl AggregateSignature {
    /// Add a signature to the aggregation.
    pub fn add(&mut self, sig: &Signature) {
        self.0.add(sig.0.clone())
    }

    /// Verify a list of messages against a list of public keys.
    pub(crate) fn verify_messages<'a>(
        &self,
        messages_and_keys: impl Iterator<Item = (Batch, &'a PublicKey)>,
    ) -> anyhow::Result<()> {
        let hashes_and_keys =
            messages_and_keys.map(|(message, key)| (message.insert().hash(), key));
        self.verify_hash(hashes_and_keys)
    }

    /// Verify a message hash against a list of public keys.
    pub(crate) fn verify_hash<'a>(
        &self,
        hashes_and_keys: impl Iterator<Item = (MsgHash, &'a PublicKey)>,
    ) -> anyhow::Result<()> {
        let bytes_and_pks = hashes_and_keys
            .map(|(hash, pk)| (hash.0.as_bytes().to_owned(), &pk.0))
            .collect::<Vec<_>>();

        let bytes_and_pks = bytes_and_pks
            .iter()
            .map(|(bytes, pk)| (&bytes[..], *pk))
            .collect::<Vec<_>>();

        self.0.verify_hash(bytes_and_pks.as_slice())
    }
}

impl ByteFmt for AggregateSignature {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        ByteFmt::decode(bytes).map(Self)
    }

    fn encode(&self) -> Vec<u8> {
        ByteFmt::encode(&self.0)
    }
}

impl TextFmt for AggregateSignature {
    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("attester:aggregate_signature:secp256k1:")?
            .decode_hex()
            .map(Self)
    }

    fn encode(&self) -> String {
        format!(
            "attester:aggregate_signature:secp256k1:{}",
            hex::encode(ByteFmt::encode(&self.0))
        )
    }
}

impl fmt::Debug for AggregateSignature {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&TextFmt::encode(self))
    }
}
