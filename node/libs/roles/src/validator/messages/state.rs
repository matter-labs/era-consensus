use anyhow::Context as _;
use zksync_protobuf::{required, ProtoFmt};

use crate::proto::validator as proto;

use super::{
    v1::{CommitQC, Phase, ReplicaCommit, TimeoutQC},
    v2::ChonkyV2State,
    Proposal, ViewNumber,
};

/// The struct that contains the replica state to be persisted.
// TODO: To be turned into an enum after we deprecate v1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplicaState {
    /// The current view number.
    pub view: ViewNumber,
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
    /// ChonkyBFT v2 state.
    pub v2: Option<ChonkyV2State>,
}

impl Default for ReplicaState {
    fn default() -> Self {
        Self {
            view: ViewNumber(0),
            phase: Phase::Prepare,
            high_vote: None,
            high_commit_qc: None,
            high_timeout_qc: None,
            proposals: vec![],
            v2: None,
        }
    }
}

impl ProtoFmt for ReplicaState {
    type Proto = proto::ReplicaState;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        use proto::replica_state::T;
        let v2 =
            r.t.as_ref()
                .map(|t| match t {
                    T::V2(v2) => ProtoFmt::read(v2).context("v2"),
                })
                .transpose()?;

        Ok(Self {
            view: ViewNumber(r.view.context("view_number")?),
            phase: read_required(&r.phase).context("phase")?,
            high_vote: read_optional(&r.high_vote).context("high_vote")?,
            high_commit_qc: read_optional(&r.high_qc).context("high_commit_qc")?,
            high_timeout_qc: read_optional(&r.high_timeout_qc).context("high_timeout_qc")?,
            proposals: r
                .proposals
                .iter()
                .map(ProtoFmt::read)
                .collect::<Result<_, _>>()
                .context("proposals")?,
            v2,
        })
    }

    fn build(&self) -> Self::Proto {
        use proto::replica_state::T;
        Self::Proto {
            view: Some(self.view.0),
            phase: Some(self.phase.build()),
            high_vote: self.high_vote.as_ref().map(|x| x.build()),
            high_qc: self.high_commit_qc.as_ref().map(|x| x.build()),
            high_timeout_qc: self.high_timeout_qc.as_ref().map(|x| x.build()),
            proposals: self.proposals.iter().map(|p| p.build()).collect(),
            t: self.v2.as_ref().map(|x| T::V2(x.build())),
        }
    }
}
