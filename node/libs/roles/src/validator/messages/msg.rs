//! Generic message types.
use std::fmt;

use anyhow::Context as _;
use zksync_consensus_crypto::{keccak256, ByteFmt, Text, TextFmt};
use zksync_consensus_utils::enum_util::{BadVariantError, Variant};
use zksync_protobuf::{read_required, required, ProtoFmt};

use super::{ConsensusMsg, NetAddress};
use crate::{node::SessionId, proto::validator as proto, validator};

/// Generic message type for a validator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Msg {
    /// consensus
    Consensus(ConsensusMsg),
    /// authentication
    SessionId(SessionId),
    /// validator discovery
    NetAddress(NetAddress),
}

impl Msg {
    /// Returns the hash of the message.
    pub fn hash(&self) -> MsgHash {
        MsgHash(keccak256::Keccak256::new(&zksync_protobuf::canonical(self)))
    }
}

impl Variant<Msg> for ConsensusMsg {
    fn insert(self) -> Msg {
        Msg::Consensus(self)
    }
    fn extract(msg: Msg) -> Result<Self, BadVariantError> {
        let Msg::Consensus(this) = msg else {
            return Err(BadVariantError);
        };
        Ok(this)
    }
}

impl Variant<Msg> for SessionId {
    fn insert(self) -> Msg {
        Msg::SessionId(self)
    }
    fn extract(msg: Msg) -> Result<Self, BadVariantError> {
        let Msg::SessionId(this) = msg else {
            return Err(BadVariantError);
        };
        Ok(this)
    }
}

impl Variant<Msg> for NetAddress {
    fn insert(self) -> Msg {
        Msg::NetAddress(self)
    }
    fn extract(msg: Msg) -> Result<Self, BadVariantError> {
        let Msg::NetAddress(this) = msg else {
            return Err(BadVariantError);
        };
        Ok(this)
    }
}

impl ProtoFmt for Msg {
    type Proto = proto::Msg;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        use proto::msg::T;
        Ok(match r.t.as_ref().context("missing")? {
            T::Consensus(r) => Self::Consensus(ProtoFmt::read(r).context("Consensus")?),
            T::SessionId(r) => Self::SessionId(SessionId(r.clone())),
            T::NetAddress(r) => Self::NetAddress(ProtoFmt::read(r).context("NetAddress")?),
        })
    }

    fn build(&self) -> Self::Proto {
        use proto::msg::T;

        let t = match self {
            Self::Consensus(x) => T::Consensus(x.build()),
            Self::SessionId(x) => T::SessionId(x.0.clone()),
            Self::NetAddress(x) => T::NetAddress(x.build()),
        };

        Self::Proto { t: Some(t) }
    }
}

/// Hash of a message.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct MsgHash(pub(crate) keccak256::Keccak256);

impl ByteFmt for MsgHash {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        ByteFmt::decode(bytes).map(Self)
    }

    fn encode(&self) -> Vec<u8> {
        ByteFmt::encode(&self.0)
    }
}

impl TextFmt for MsgHash {
    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("validator_msg:keccak256:")?
            .decode_hex()
            .map(Self)
    }

    fn encode(&self) -> String {
        format!(
            "validator_msg:keccak256:{}",
            hex::encode(ByteFmt::encode(&self.0))
        )
    }
}

impl ProtoFmt for MsgHash {
    type Proto = proto::MsgHash;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(ByteFmt::decode(required(&r.keccak256)?)?))
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            keccak256: Some(self.0.encode()),
        }
    }
}

impl fmt::Debug for MsgHash {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.write_str(&TextFmt::encode(self))
    }
}

/// Strongly typed signed message.
/// WARNING: signature is not guaranteed to be valid.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Signed<V: Variant<Msg>> {
    /// The message that was signed.
    pub msg: V,
    /// The public key of the signer.
    pub key: validator::PublicKey,
    /// The signature.
    pub sig: validator::Signature,
}

impl<V: Variant<Msg> + Clone> Signed<V> {
    /// Verify the signature on the message.
    pub fn verify(&self) -> anyhow::Result<()> {
        self.sig.verify_msg(&self.msg.clone().insert(), &self.key)
    }
}

impl<V: Variant<Msg>> Signed<V> {
    /// Casts a signed message variant to sub/super variant.
    /// It is an equivalent of constructing/deconstructing enum values.
    pub fn cast<U: Variant<Msg>>(self) -> Result<Signed<U>, BadVariantError> {
        Ok(Signed {
            msg: U::extract(self.msg.insert())?,
            key: self.key,
            sig: self.sig,
        })
    }
}

impl<V: Variant<Msg> + Clone> ProtoFmt for Signed<V> {
    type Proto = proto::Signed;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            msg: V::extract(read_required::<Msg>(&r.msg).context("msg")?)?,
            key: read_required(&r.key).context("key")?,
            sig: read_required(&r.sig).context("sig")?,
        })
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            msg: Some(self.msg.clone().insert().build()),
            key: Some(self.key.build()),
            sig: Some(self.sig.build()),
        }
    }
}
