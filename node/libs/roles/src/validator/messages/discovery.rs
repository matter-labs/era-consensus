use std::net;
use zksync_concurrency::time;

/// A message broadcasted by a validator
/// over the gossip network announcing
/// its own TCP address. See `schema/proto/roles/validator.proto`.
/// The NetAddress message with highest (version,timestamp) is
/// considered to be the newest (compared lexicographically).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NetAddress {
    /// Address of the validator.
    pub addr: net::SocketAddr,
    /// Version of the discovery announcement.
    pub version: u64,
    /// Time at which this message has been signed.
    pub timestamp: time::Utc,
}

/// Protobuf encoded NetAddress.
/// It is used to avoid reencoding data and
/// therefore losing the unknown fields.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EncodedNetAddress(NetAddress, Vec<u8>);

impl std::ops::Deref for EncodedNetAddress {
    type Target = NetAddress;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<NetAddress> for EncodedNetAddress {
    fn from(x: NetAddress) -> Self {
        let bytes = zksync_protobuf::canonical(&x);
        Self(x, bytes)
    }
}

impl EncodedNetAddress {
    /// Constructs EncodedNetAddress from
    /// encoded representation.
    pub fn new(bytes: Vec<u8>) -> anyhow::Result<Self> {
        let x = zksync_protobuf::decode(&bytes)?;
        Ok(Self(x, bytes))
    }
    /// Reference to encoded representation.
    pub fn encoded(&self) -> &[u8] {
        &self.1
    }
}

impl NetAddress {
    /// Checks if `self` is a newer version than `b`.
    pub fn is_newer(&self, b: &Self) -> bool {
        (self.version, self.timestamp) > (b.version, b.timestamp)
    }
}
