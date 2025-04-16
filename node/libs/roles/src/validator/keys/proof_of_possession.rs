use std::fmt;

use zksync_consensus_crypto::{bls12_381, ByteFmt, Text, TextFmt};

use super::PublicKey;

/// Proof of possession of a validator secret key.
#[derive(Clone, PartialEq, Eq)]
pub struct ProofOfPossession(pub(crate) bls12_381::ProofOfPossession);

impl ProofOfPossession {
    /// Verifies the proof against the public key.
    pub fn verify(&self, pk: &PublicKey) -> anyhow::Result<()> {
        self.0.verify(&pk.0)
    }
}

impl fmt::Debug for ProofOfPossession {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.write_str(&TextFmt::encode(self))
    }
}

impl ByteFmt for ProofOfPossession {
    fn encode(&self) -> Vec<u8> {
        ByteFmt::encode(&self.0)
    }
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        ByteFmt::decode(bytes).map(Self)
    }
}

impl TextFmt for ProofOfPossession {
    fn encode(&self) -> String {
        format!(
            "validator:pop:bls12_381:{}",
            hex::encode(ByteFmt::encode(&self.0))
        )
    }
    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("validator:pop:bls12_381:")?
            .decode_hex()
            .map(Self)
    }
}
