use super::CapabilityId;
use anyhow::Context as _;
use std::collections::HashMap;
use zksync_consensus_schema as schema;
use zksync_consensus_schema::{proto::network::mux as proto, required};

pub(super) struct Handshake {
    /// Maximal supported number of the accept streams per capability.
    pub(super) accept_max_streams: HashMap<CapabilityId, u32>,
    /// Maximal supported number of the connect streams per capability.
    pub(super) connect_max_streams: HashMap<CapabilityId, u32>,
}

fn read_max_streams(
    capabilities: &Vec<proto::handshake::Capability>,
) -> anyhow::Result<HashMap<CapabilityId, u32>> {
    let mut ms = HashMap::new();
    for r in capabilities {
        let id = *required(&r.id).context("id")?;
        let max_streams = *required(&r.max_streams).context("max_streams")?;
        if ms.insert(id, max_streams).is_some() {
            anyhow::bail!("duplicate entry for id {id}");
        }
    }
    Ok(ms)
}

fn build_capabilities(
    max_streams: &HashMap<CapabilityId, u32>,
) -> Vec<proto::handshake::Capability> {
    max_streams
        .iter()
        .map(|(id, max_streams)| proto::handshake::Capability {
            id: Some(*id),
            max_streams: Some(*max_streams),
        })
        .collect()
}

impl schema::ProtoFmt for Handshake {
    type Proto = proto::Handshake;

    fn max_size() -> usize {
        schema::kB
    }

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            accept_max_streams: read_max_streams(&r.accept).context("accept")?,
            connect_max_streams: read_max_streams(&r.connect).context("connect")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            accept: build_capabilities(&self.accept_max_streams),
            connect: build_capabilities(&self.connect_max_streams),
        }
    }
}
