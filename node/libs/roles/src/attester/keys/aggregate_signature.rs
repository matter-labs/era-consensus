use std::fmt;

use zksync_consensus_crypto::{bls12_381, ByteFmt, Text, TextFmt};
use zksync_consensus_utils::enum_util::Variant;

use crate::attester::{Batch, MsgHash, Signers};

// TODO: Once EIP-2537 is merged, `attester::Signature` could be changed to point at the BLS signature.
//       For now these are just a placeholders so we can keep an `AggregateSignature` around.
type BlsSignature = bls12_381::Signature;
type BlsPublicKey = bls12_381::PublicKey;

/// The index of the attester in the committee.
type AttesterIndex = usize;

/// The aggregate signature of attesters over the same message.
///
/// TODO: Once EIP-2537 is merged, this can replace the Secp256k1 `MultiSig` in the `Batch`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct AggregateSignature(pub(crate) bls12_381::AggregateSignature);

impl AggregateSignature {
    /// Add a signature to the aggregation.
    pub fn add(&mut self, sig: &bls12_381::Signature) {
        self.0.add(sig)
    }

    /// Verify a message against a list of public keys.
    pub(crate) fn verify_msg<'a>(
        &self,
        message: Batch,
        keys: impl Iterator<Item = &'a BlsPublicKey>,
    ) -> anyhow::Result<()> {
        self.verify_hash(&message.insert().hash(), keys)
    }

    /// Verify a message hash against a list of public keys.
    pub(crate) fn verify_hash<'a>(
        &self,
        hash: &MsgHash,
        keys: impl Iterator<Item = &'a BlsPublicKey>,
    ) -> anyhow::Result<()> {
        self.0
            .verify(keys.map(|pk| (hash.0.as_bytes().as_slice(), pk)))
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
        text.strip("attester:aggregate_signature:bls12_381:")?
            .decode_hex()
            .map(Self)
    }

    fn encode(&self) -> String {
        format!(
            "attester:aggregate_signature:bls12_381:{}",
            hex::encode(ByteFmt::encode(&self.0))
        )
    }
}

impl fmt::Debug for AggregateSignature {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&TextFmt::encode(self))
    }
}

/// A BLS multi-sig, which is an aggregated signature and a bitvector
/// indicating which members of the committee have signed.
///
/// (By contrast a BLS threshold signature would also be an aggregate
/// but wouldn't require the bit vector).
pub struct AggregateMultiSig {
    /// Bitvector indicating which attesters are part of the aggregate.
    pub(crate) signers: Signers,
    /// Aggregated BLS signature.
    pub(crate) sig: AggregateSignature,
}

impl AggregateMultiSig {
    /// Check whether an attester has added their signature.
    ///
    /// Returns false if the index is out of range.
    pub fn contains(&self, idx: AttesterIndex) -> bool {
        self.signers.0.get(idx).unwrap_or_default()
    }

    /// Add an attester signature.
    ///
    /// Panics if the index is out of range.
    pub fn add(&mut self, idx: AttesterIndex, sig: BlsSignature) {
        self.signers.0.set(idx, true);
        self.sig.add(&sig)
    }

    /// Verify a message against a list of public keys in the attester committee,
    /// if they are part of the ones who actually submitted their signatures.
    pub(crate) fn verify_msg<'a>(
        &self,
        message: Batch,
        keys: impl Iterator<Item = (AttesterIndex, &'a BlsPublicKey)>,
    ) -> anyhow::Result<()> {
        self.verify_hash(&message.insert().hash(), keys)
    }

    /// Verify a message hash against a list of public keys in the attester committee,
    /// if they are part of the ones who actually submitted their signatures.
    pub(crate) fn verify_hash<'a>(
        &self,
        hash: &MsgHash,
        keys: impl Iterator<Item = (AttesterIndex, &'a BlsPublicKey)>,
    ) -> anyhow::Result<()> {
        self.sig.verify_hash(
            hash,
            keys.filter_map(|(idx, pk)| if self.contains(idx) { Some(pk) } else { None }),
        )
    }
}
