use super::{Error, PublicKey, Signature};
use crate::validator::messages::{Msg, MsgHash};
use crypto::{bn254, ByteFmt, Text, TextFmt};
use std::fmt;
use utils::enum_util::Variant;

/// An aggregate signature from a validator.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AggregateSignature(pub(crate) bn254::AggregateSignature);

impl AggregateSignature {
    /// Generate  a new aggregate signature from a list of signatures.
    pub fn aggregate<'a>(sigs: impl IntoIterator<Item = &'a Signature>) -> Self {
        AggregateSignature(bn254::AggregateSignature::aggregate(
            sigs.into_iter().map(|sig| &sig.0).collect::<Vec<_>>(),
        ))
    }

    /// Verify a list of messages against a list of public keys.
    pub(crate) fn verify_messages<'a, V: Variant<Msg>>(
        &self,
        messages_and_keys: impl Iterator<Item = (V, &'a PublicKey)>,
    ) -> Result<(), Error> {
        let hashes_and_keys =
            messages_and_keys.map(|(message, key)| (message.insert().hash(), key));
        self.verify_hash(hashes_and_keys)
    }

    /// Verify a message hash against a list of public keys.
    pub(crate) fn verify_hash<'a>(
        &self,
        hashes_and_keys: impl Iterator<Item = (MsgHash, &'a PublicKey)>,
    ) -> Result<(), Error> {
        let bytes_and_pks: Vec<_> = hashes_and_keys
            .map(|(hash, pk)| (hash.0.as_bytes().to_owned(), &pk.0))
            .collect();

        let bytes_and_pks = bytes_and_pks.iter().map(|(bytes, pk)| (&bytes[..], *pk));

        self.0.verify(bytes_and_pks)
    }
}

impl ByteFmt for AggregateSignature {
    fn encode(&self) -> Vec<u8> {
        ByteFmt::encode(&self.0)
    }
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        ByteFmt::decode(bytes).map(Self)
    }
}

impl TextFmt for AggregateSignature {
    fn encode(&self) -> String {
        format!(
            "validator:aggregate_signature:bn254:{}",
            hex::encode(ByteFmt::encode(&self.0))
        )
    }
    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("validator:aggregate_signature:bn254:")?
            .decode_hex()
            .map(Self)
    }
}

impl fmt::Debug for AggregateSignature {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        writeln!(fmt, "Signature: {}", hex::encode(ByteFmt::encode(&self.0)))
    }
}
