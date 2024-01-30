//! Cryptographic keys representing the node role.
//! Public key identifies a single node in the gossip network.
//! Each node must have a different key.

use super::{Msg, MsgHash, Signed};
pub use ed25519::InvalidSignatureError;
use std::{fmt, sync::Arc};
use zksync_consensus_crypto::{ed25519, ByteFmt, Text, TextFmt};
use zksync_consensus_utils::enum_util::Variant;

/// A node's secret key.
#[derive(Clone)]
pub struct SecretKey(pub(super) Arc<ed25519::SecretKey>);

impl SecretKey {
    /// Generates a secret key from a cryptographically-secure entropy source.
    pub fn generate() -> Self {
        Self(Arc::new(ed25519::SecretKey::generate()))
    }

    /// Sign a message hash.
    pub fn sign(&self, msg_hash: &MsgHash) -> Signature {
        Signature(self.0.sign(&ByteFmt::encode(msg_hash)))
    }

    /// Sign a message.
    pub fn sign_msg<V: Variant<Msg>>(&self, msg: V) -> Signed<V> {
        let msg = msg.insert();
        Signed {
            key: self.public(),
            sig: self.sign(&msg.hash()),
            msg: V::extract(msg).unwrap(),
        }
    }

    /// Get the public key corresponding to this secret key.
    pub fn public(&self) -> PublicKey {
        PublicKey(self.0.public())
    }
}

impl ByteFmt for SecretKey {
    fn encode(&self) -> Vec<u8> {
        ByteFmt::encode(&*self.0)
    }
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        ByteFmt::decode(bytes).map(|k| Self(Arc::new(k)))
    }
}

impl TextFmt for SecretKey {
    fn encode(&self) -> String {
        format!("node:secret:ed25519:{}", hex::encode(ByteFmt::encode(self)))
    }
    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("node:secret:ed25519:")?.decode_hex()
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        write!(fmt, "<secret for {}>", TextFmt::encode(&self.public()))
    }
}

/// A node's public key.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PublicKey(pub(super) ed25519::PublicKey);

impl PublicKey {
    /// Verify a signed message.
    pub fn verify(&self, msg_hash: &MsgHash, sig: &Signature) -> Result<(), InvalidSignatureError> {
        self.0.verify(&ByteFmt::encode(msg_hash), &sig.0)
    }
}

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
        format!("node:public:ed25519:{}", hex::encode(ByteFmt::encode(self)))
    }
    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("node:public:ed25519:")?.decode_hex()
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.write_str(&TextFmt::encode(self))
    }
}

/// A signature of a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature(pub(super) ed25519::Signature);

impl ByteFmt for Signature {
    fn encode(&self) -> Vec<u8> {
        ByteFmt::encode(&self.0)
    }
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        ByteFmt::decode(bytes).map(Self)
    }
}
