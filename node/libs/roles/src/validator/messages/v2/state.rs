use anyhow::Context as _;
use zksync_protobuf::{read_optional, read_required, ProtoFmt};

use super::{CommitQC, Phase, ReplicaCommit, TimeoutQC};
use crate::{
    proto::validator as proto,
    validator::{Proposal, ViewNumber},
};

/// The struct that contains the state of a ChonkyBFT v2 replica to be persisted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChonkyV2State {
    /// The current view number.
    pub view_number: ViewNumber,
    /// The current phase.
    pub phase: Phase,
    /// The highest block proposal that the replica has committed to.
    pub high_vote: Option<ReplicaCommit>,
    /// The highest commit quorum certificate known to the replica.
    pub high_commit_qc: Option<CommitQC>,
    /// The highest timeout quorum certificate known to the replica.
    pub high_timeout_qc: Option<TimeoutQC>,
    /// A cache of the received block proposals.
    pub proposals: Vec<Proposal>,
}

impl Default for ChonkyV2State {
    fn default() -> Self {
        Self {
            view_number: ViewNumber(0),
            phase: Phase::Prepare,
            high_vote: None,
            high_commit_qc: None,
            high_timeout_qc: None,
            proposals: vec![],
        }
    }
}

impl ProtoFmt for ChonkyV2State {
    type Proto = proto::ChonkyV2State;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            view_number: ViewNumber(r.view_number.context("view_number")?),
            phase: read_required(&r.phase).context("phase")?,
            high_vote: read_optional(&r.high_vote).context("high_vote")?,
            high_commit_qc: read_optional(&r.high_commit_qc).context("high_commit_qc")?,
            high_timeout_qc: read_optional(&r.high_timeout_qc).context("high_timeout_qc")?,
            proposals: r
                .proposals
                .iter()
                .map(ProtoFmt::read)
                .collect::<Result<_, _>>()
                .context("proposals")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            view_number: Some(self.view_number.0),
            phase: Some(self.phase.build()),
            high_vote: self.high_vote.as_ref().map(|x| x.build()),
            high_commit_qc: self.high_commit_qc.as_ref().map(|x| x.build()),
            high_timeout_qc: self.high_timeout_qc.as_ref().map(|x| x.build()),
            proposals: self.proposals.iter().map(|p| p.build()).collect(),
        }
    }
}
