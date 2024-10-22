//! Defines storage layer for persistent replica state.
use crate::proto;
use anyhow::Context as _;
use std::fmt;
use zksync_concurrency::ctx;
use zksync_consensus_roles::validator;
use zksync_protobuf::{read_optional, read_required, required, ProtoFmt};

/// Storage for [`ReplicaState`].
///
/// Implementations **must** propagate context cancellation using [`StorageError::Canceled`].
#[async_trait::async_trait]
pub trait ReplicaStore: 'static + fmt::Debug + Send + Sync {
    /// Gets the replica state, if it is contained in the database.
    async fn state(&self, ctx: &ctx::Ctx) -> ctx::Result<ReplicaState>;

    /// Stores the given replica state into the database.
    async fn set_state(&self, ctx: &ctx::Ctx, state: &ReplicaState) -> ctx::Result<()>;
}

/// A payload of a proposed block which is not known to be finalized yet.
/// Replicas have to persist such proposed payloads for liveness:
/// consensus may finalize a block without knowing a payload in case of reproposals.
/// Currently we do not store the BlockHeader, because it is always
///   available in the LeaderPrepare message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proposal {
    /// Number of a block for which this payload has been proposed.
    pub number: validator::BlockNumber,
    /// Proposed payload.
    pub payload: validator::Payload,
}

/// The struct that contains the replica state to be persisted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplicaState {
    /// The current view number.
    pub view: validator::ViewNumber,
    /// The current phase.
    pub phase: validator::Phase,
    /// The highest block proposal that the replica has committed to.
    pub high_vote: Option<validator::ReplicaCommit>,
    /// The highest commit quorum certificate known to the replica.
    pub high_commit_qc: Option<validator::CommitQC>,
    /// The highest timeout quorum certificate known to the replica.
    pub high_timeout_qc: Option<validator::TimeoutQC>,
    /// A cache of the received block proposals.
    pub proposals: Vec<Proposal>,
}

impl Default for ReplicaState {
    fn default() -> Self {
        Self {
            view: validator::ViewNumber(0),
            phase: validator::Phase::Prepare,
            high_vote: None,
            high_commit_qc: None,
            high_timeout_qc: None,
            proposals: vec![],
        }
    }
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

impl ProtoFmt for ReplicaState {
    type Proto = proto::ReplicaState;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            view: validator::ViewNumber(r.view.context("view_number")?),
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
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            view: Some(self.view.0),
            phase: Some(self.phase.build()),
            high_vote: self.high_vote.as_ref().map(|x| x.build()),
            high_qc: self.high_commit_qc.as_ref().map(|x| x.build()),
            high_timeout_qc: self.high_timeout_qc.as_ref().map(|x| x.build()),
            proposals: self.proposals.iter().map(|p| p.build()).collect(),
        }
    }
}
