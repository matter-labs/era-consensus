use std::fmt;

use zksync_consensus_crypto::{bls12_381, ByteFmt, Text, TextFmt};
use zksync_protobuf::{required, ProtoFmt};

use super::PublicKey;
use crate::{
    proto::validator as proto,
    validator::messages::{Msg, MsgHash},
};

/// A signature from a validator.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Signature(pub(crate) bls12_381::Signature);

impl Signature {
    /// Verify a message against a public key.
    pub fn verify_msg(&self, msg: &Msg, pk: &PublicKey) -> anyhow::Result<()> {
        self.verify_hash(&msg.hash(), pk)
    }

    /// Verify a message hash against a public key.
    pub fn verify_hash(&self, msg_hash: &MsgHash, pk: &PublicKey) -> anyhow::Result<()> {
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
            "validator:signature:bls12_381:{}",
            hex::encode(ByteFmt::encode(&self.0))
        )
    }
    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("validator:signature:bls12_381:")?
            .decode_hex()
            .map(Self)
    }
}

impl ProtoFmt for Signature {
    type Proto = proto::Signature;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(ByteFmt::decode(required(&r.bn254)?)?))
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            bn254: Some(self.0.encode()),
        }
    }
}

impl fmt::Debug for Signature {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.write_str(&TextFmt::encode(self))
    }
}
