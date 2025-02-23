//! Defines RPC for passing consensus messages.
use std::sync::Arc;

use anyhow::Context as _;
use zksync_consensus_roles::attester;
use zksync_protobuf::{read_optional, ProtoFmt};

use super::Capability;
use crate::proto::gossip as proto;

/// RPC pushing fresh batch votes.
pub(crate) struct Rpc;

impl super::Rpc for Rpc {
    const CAPABILITY: Capability = Capability::PushBatchVotes;
    const INFLIGHT: u32 = 1;
    const METHOD: &'static str = "push_batch_votes";
    type Req = Req;
    type Resp = Resp;
}

/// Signed batch message that the receiving peer should process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Req {
    /// Requesting the peer to respond with votes for the batch.
    pub(crate) want_votes_for: Option<attester::Batch>,
    /// New votes that server might be not aware of.
    pub(crate) votes: Vec<Arc<attester::Signed<attester::Batch>>>,
}

pub(crate) struct Resp {
    /// Votes requested by the peer.
    pub(crate) votes: Vec<Arc<attester::Signed<attester::Batch>>>,
}

impl ProtoFmt for Req {
    type Proto = proto::PushBatchVotes;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        let mut votes = vec![];
        for (i, e) in r.votes.iter().enumerate() {
            votes.push(Arc::new(
                ProtoFmt::read(e).with_context(|| format!("votes[{i}]"))?,
            ));
        }
        Ok(Self {
            want_votes_for: read_optional(&r.want_votes_for).context("want_votes_for")?,
            votes,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            want_votes_for: self.want_votes_for.as_ref().map(ProtoFmt::build),
            votes: self
                .votes
                .iter()
                .map(|a| ProtoFmt::build(a.as_ref()))
                .collect(),
        }
    }
}

impl ProtoFmt for Resp {
    type Proto = proto::PushBatchVotesResp;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        let mut votes = vec![];
        for (i, e) in r.votes.iter().enumerate() {
            votes.push(Arc::new(
                ProtoFmt::read(e).with_context(|| format!("votes[{i}]"))?,
            ));
        }
        Ok(Self { votes })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            votes: self
                .votes
                .iter()
                .map(|a| ProtoFmt::build(a.as_ref()))
                .collect(),
        }
    }
}
