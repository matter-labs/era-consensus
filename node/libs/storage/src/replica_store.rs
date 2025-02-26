//! Defines storage layer for persistent replica state.
use std::fmt;

use anyhow::Context as _;
use zksync_concurrency::ctx;
use zksync_consensus_roles::validator;
use zksync_protobuf::{read_optional, read_required, required, ProtoFmt};

use crate::proto;

/// Storage for [`ReplicaState`].
///
/// Implementations **must** propagate context cancellation using [`StorageError::Canceled`].
#[async_trait::async_trait]
pub trait ReplicaStore: 'static + fmt::Debug + Send + Sync {
    /// Gets the replica state, if it is contained in the database. Otherwise, returns the default
    /// state.
    async fn state(&self, ctx: &ctx::Ctx) -> ctx::Result<ReplicaState>;

    /// Stores the given replica state into the database.
    async fn set_state(&self, ctx: &ctx::Ctx, state: &ReplicaState) -> ctx::Result<()>;
}

/// The struct that contains the replica state to be persisted.
// TODO: To be turned into an enum after we deprecate v1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplicaState {
    /// The current view number.
    pub view: validator::v1::ViewNumber,
    /// The current phase.
    pub phase: validator::v1::Phase,
    /// The highest block proposal that the replica has committed to.
    pub high_vote: Option<validator::v1::ReplicaCommit>,
    /// The highest commit quorum certificate known to the replica.
    pub high_commit_qc: Option<validator::v1::CommitQC>,
    /// The highest timeout quorum certificate known to the replica.
    pub high_timeout_qc: Option<validator::v1::TimeoutQC>,
    /// A cache of the received block proposals.
    pub proposals: Vec<Proposal>,
    /// ChonkyBFT v2 state.
    pub v2: Option<ChonkyV2State>,
}

impl Default for ReplicaState {
    fn default() -> Self {
        Self {
            view: validator::v1::ViewNumber(0),
            phase: validator::v1::Phase::Prepare,
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
            view: validator::v1::ViewNumber(r.view.context("view_number")?),
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

/// The struct that contains the state of a ChonkyBFT v2 replica to be persisted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChonkyV2State {
    /// The current view number.
    pub view_number: validator::v2::ViewNumber,
    /// The current epoch number.
    pub epoch_number: validator::v2::EpochNumber,
    /// The current phase.
    pub phase: validator::v2::Phase,
    /// The highest block proposal that the replica has committed to.
    pub high_vote: Option<validator::v2::ReplicaCommit>,
    /// The highest commit quorum certificate known to the replica.
    pub high_commit_qc: Option<validator::v2::CommitQC>,
    /// The highest timeout quorum certificate known to the replica.
    pub high_timeout_qc: Option<validator::v2::TimeoutQC>,
    /// A cache of the received block proposals.
    pub proposals: Vec<Proposal>,
}

impl Default for ChonkyV2State {
    fn default() -> Self {
        Self {
            view_number: validator::v2::ViewNumber(0),
            epoch_number: validator::v2::EpochNumber(0),
            phase: validator::v2::Phase::Prepare,
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
            view_number: validator::v2::ViewNumber(r.view_number.context("view_number")?),
            epoch_number: validator::v2::EpochNumber(r.epoch_number.context("epoch_number")?),
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
            epoch_number: Some(self.epoch_number.0),
            phase: Some(self.phase.build()),
            high_vote: self.high_vote.as_ref().map(|x| x.build()),
            high_commit_qc: self.high_commit_qc.as_ref().map(|x| x.build()),
            high_timeout_qc: self.high_timeout_qc.as_ref().map(|x| x.build()),
            proposals: self.proposals.iter().map(|p| p.build()).collect(),
        }
    }
}

/// A payload of a proposed block which is not known to be finalized yet.
/// Replicas have to persist such proposed payloads for liveness:
/// consensus may finalize a block without knowing a payload in case of reproposals.
/// Currently we do not store the BlockHeader, because it is always
/// available in the LeaderPrepare message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proposal {
    /// Number of a block for which this payload has been proposed.
    pub number: validator::BlockNumber,
    /// Proposed payload.
    pub payload: validator::Payload,
}

impl ProtoFmt for Proposal {
    type Proto = proto::Proposal;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            number: validator::BlockNumber(*required(&r.number).context("number")?),
            payload: validator::Payload(required(&r.payload).context("payload")?.clone()),
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            number: Some(self.number.0),
            payload: Some(self.payload.0.clone()),
        }
    }
}
