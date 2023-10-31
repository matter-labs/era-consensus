//! Generic message types.

use super::{ConsensusMsg, NetAddress};
use crate::{validator::Error, node::SessionId, validator};
use crypto::{sha256, ByteFmt, Text, TextFmt};
use std::fmt;
use utils::enum_util::{BadVariantError, Variant};

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
        MsgHash(sha256::Sha256::new(&schema::canonical(self)))
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

/// Hash of a message.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MsgHash(pub(crate) sha256::Sha256);

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
        text.strip("validator_msg:sha256:")?.decode_hex().map(Self)
    }

    fn encode(&self) -> String {
        format!(
            "validator_msg:sha256:{}",
            hex::encode(ByteFmt::encode(&self.0))
        )
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
    pub fn verify(&self) -> Result<(), Error> {
        self.sig.verify_msg(&self.msg.clone().insert(), &self.key)
    }
}

impl<V: Variant<Msg>> Signed<V> {
    /// Casts a signed message variant to sub/super variant.
    /// It is an equivalent of constructing/deconstructing enum values.
    pub fn cast<V2: Variant<Msg>>(self) -> Result<Signed<V2>, BadVariantError> {
        Ok(Signed {
            msg: V2::extract(self.msg.insert())?,
            key: self.key,
            sig: self.sig,
        })
    }
}
