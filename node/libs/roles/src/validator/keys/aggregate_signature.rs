use std::fmt;

use zksync_consensus_crypto::{bls12_381, ByteFmt, Text, TextFmt};
use zksync_consensus_utils::enum_util::Variant;

use super::{PublicKey, Signature};
use crate::validator::messages::{Msg, MsgHash};

use crate::proto::validator as proto;
use zksync_protobuf::{required, ProtoFmt};

/// An aggregate signature from a validator.
/// WARNING: any change to this struct may invalidate preexisting signatures. See `TimeoutQC` docs.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct AggregateSignature(pub(crate) bls12_381::AggregateSignature);

impl AggregateSignature {
    /// Add a signature to the aggregation.
    pub fn add(&mut self, sig: &Signature) {
        self.0.add(&sig.0)
    }

    /// Generate a new aggregate signature from a list of signatures.
    pub fn aggregate<'a>(sigs: impl IntoIterator<Item = &'a Signature>) -> Self {
        let mut agg = Self::default();
        for sig in sigs {
            agg.add(sig);
        }
        agg
    }

    /// Verify a list of messages against a list of public keys.
    pub(crate) fn verify_messages<'a, V: Variant<Msg>>(
        &self,
        messages_and_keys: impl Iterator<Item = (V, &'a PublicKey)>,
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
        let bytes_and_pks: Vec<_> = hashes_and_keys
            .map(|(hash, pk)| (hash.0.as_bytes().to_owned(), &pk.0))
            .collect();

        let bytes_and_pks = bytes_and_pks.iter().map(|(bytes, pk)| (&bytes[..], *pk));

        self.0.verify(bytes_and_pks)
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
        text.strip("validator:aggregate_signature:bls12_381:")?
            .decode_hex()
            .map(Self)
    }

    fn encode(&self) -> String {
        format!(
            "validator:aggregate_signature:bls12_381:{}",
            hex::encode(ByteFmt::encode(&self.0))
        )
    }
}

impl ProtoFmt for AggregateSignature {
    type Proto = proto::AggregateSignature;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(ByteFmt::decode(required(&r.bn254)?)?))
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            bn254: Some(self.0.encode()),
        }
    }
}

impl fmt::Debug for AggregateSignature {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&TextFmt::encode(self))
    }
}
