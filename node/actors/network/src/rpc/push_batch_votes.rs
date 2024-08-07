//! Defines RPC for passing consensus messages.
use super::Capability;
use crate::proto::gossip as proto;
use anyhow::Context as _;
use std::sync::Arc;
use zksync_consensus_roles::attester;
use zksync_protobuf::ProtoFmt;

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
    pub(crate) want_snapshot: Option<bool>,
    pub(crate) votes: Vec<Arc<attester::Signed<attester::Batch>>>,
}

pub(crate) struct Resp {
    // Empty if `req.want_snapshot` was not set.
    pub(crate) votes: Vec<Arc<attester::Signed<attester::Batch>>>,
}

impl Req {
    pub(crate) fn want_snapshot(&self) -> bool {
        self.want_snapshot.unwrap_or(false)
    }
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
            want_snapshot: r.want_snapshot,
            votes,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            want_snapshot: self.want_snapshot,
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
