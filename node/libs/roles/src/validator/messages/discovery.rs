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

impl NetAddress {
    /// Checks if `self` is a newer version than `b`.
    pub fn is_newer(&self, b: &Self) -> bool {
        (self.version, self.timestamp) > (b.version, b.timestamp)
    }
}
