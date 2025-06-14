use anyhow::Context as _;

use zksync_consensus_crypto::{keccak256::Keccak256, ByteFmt, Text, TextFmt};
use zksync_consensus_utils::enum_util::{BadVariantError, Variant};
use zksync_protobuf::{read_required, required, ProtoFmt};

use crate::{node, proto::node as proto};

/// The ID for an authentication session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionId(pub Vec<u8>);

/// A message that can be sent between nodes.
#[allow(missing_docs)]
#[derive(Debug)]
pub enum Msg {
    // Authentication
    SessionId(SessionId),
}

impl Msg {
    /// Get the hash of this message.
    pub fn hash(&self) -> MsgHash {
        MsgHash(Keccak256::new(&zksync_protobuf::canonical(self)))
    }
}

impl Variant<Msg> for SessionId {
    fn insert(self) -> Msg {
        Msg::SessionId(self)
    }
    fn extract(msg: Msg) -> Result<Self, BadVariantError> {
        let Msg::SessionId(this) = msg;
        Ok(this)
    }
}

impl ProtoFmt for Msg {
    type Proto = proto::Msg;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        use proto::msg::T;
        Ok(match required(&r.t)? {
            T::SessionId(r) => Self::SessionId(SessionId(r.clone())),
        })
    }
    fn build(&self) -> Self::Proto {
        use proto::msg::T;
        let t = match self {
            Self::SessionId(x) => T::SessionId(x.0.clone()),
        };
        Self::Proto { t: Some(t) }
    }
}

/// Strongly typed signed message.
/// WARNING: signature is not guaranteed to be valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signed<V: Variant<Msg>> {
    /// The message that was signed.
    pub msg: V,
    /// The public key of the signer.
    pub key: node::PublicKey,
    /// The signature.
    pub sig: node::Signature,
}

impl<V: Variant<Msg> + Clone> Signed<V> {
    /// Verify the signature on the message.
    pub fn verify(&self) -> Result<(), node::InvalidSignatureError> {
        self.key
            .verify(&self.msg.clone().insert().hash(), &self.sig)
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

/// The hash of a message.
pub struct MsgHash(pub(crate) Keccak256);

impl ByteFmt for MsgHash {
    fn encode(&self) -> Vec<u8> {
        ByteFmt::encode(&self.0)
    }
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        ByteFmt::decode(bytes).map(Self)
    }
}

impl TextFmt for MsgHash {
    fn encode(&self) -> String {
        format!(
            "validator_msg:keccak256:{}",
            hex::encode(ByteFmt::encode(&self.0))
        )
    }
    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("validator_msg:keccak256:")?
            .decode_hex()
            .map(Self)
    }
}
