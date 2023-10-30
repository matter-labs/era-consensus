use crate::node;
use crypto::{sha256, ByteFmt, Text, TextFmt};
use utils::enum_util::{ErrBadVariant, Variant};

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
        MsgHash(sha256::Sha256::new(&protobuf_utils::canonical(self)))
    }
}

impl Variant<Msg> for SessionId {
    fn insert(self) -> Msg {
        Msg::SessionId(self)
    }
    fn extract(msg: Msg) -> Result<Self, ErrBadVariant> {
        let Msg::SessionId(this) = msg;
        Ok(this)
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
    pub fn verify(&self) -> Result<(), node::ErrInvalidSignature> {
        self.key
            .verify(&self.msg.clone().insert().hash(), &self.sig)
    }
}

/// The hash of a message.
pub struct MsgHash(pub(super) sha256::Sha256);

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
            "validator_msg:sha256:{}",
            hex::encode(ByteFmt::encode(&self.0))
        )
    }
    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("validator_msg:sha256:")?.decode_hex().map(Self)
    }
}
