use std::fmt;
use zksync_consensus_crypto::{bls12_381, ByteFmt, Text, TextFmt};

/// A public key for a validator.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PublicKey(pub(crate) bls12_381::PublicKey);

impl ByteFmt for PublicKey {
    fn encode(&self) -> Vec<u8> {
        ByteFmt::encode(&self.0)
    }
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        ByteFmt::decode(bytes).map(Self)
    }
}

impl TextFmt for PublicKey {
    fn encode(&self) -> String {
        format!(
            "validator:public:bls12_381:{}",
            hex::encode(ByteFmt::encode(&self.0))
        )
    }
    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("validator:public:bls12_381:")?
            .decode_hex()
            .map(Self)
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.write_str(&TextFmt::encode(self))
    }
}
