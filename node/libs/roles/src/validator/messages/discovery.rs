use std::net;

use zksync_concurrency::time;

use crate::proto::validator as proto;
use anyhow::Context as _;
use zksync_protobuf::{read_required, required, ProtoFmt};

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

impl ProtoFmt for NetAddress {
    type Proto = proto::NetAddress;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            addr: read_required(&r.addr).context("addr")?,
            version: *required(&r.version).context("version")?,
            timestamp: read_required(&r.timestamp).context("timestamp")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            addr: Some(self.addr.build()),
            version: Some(self.version),
            timestamp: Some(self.timestamp.build()),
        }
    }
}
