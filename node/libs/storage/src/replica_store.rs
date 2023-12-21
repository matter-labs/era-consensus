//! Defines storage layer for persistent replica state.
use crate::proto;
use std::fmt;
use anyhow::Context as _;
use zksync_concurrency::ctx;
use zksync_consensus_roles::validator;
use zksync_protobuf::{read_required, required, ProtoFmt};

/// Storage for [`ReplicaState`].
///
/// Implementations **must** propagate context cancellation using [`StorageError::Canceled`].
#[async_trait::async_trait]
pub trait ReplicaStore: fmt::Debug + Send + Sync {
    /// Gets the replica state, if it is contained in the database.
    async fn state(&self, ctx: &ctx::Ctx) -> ctx::Result<Option<ReplicaState>>;

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
    pub high_vote: validator::ReplicaCommit,
    /// The highest commit quorum certificate known to the replica.
    pub high_qc: validator::CommitQC,
    /// A cache of the received block proposals.
    pub proposals: Vec<Proposal>,
}

impl From<validator::CommitQC> for ReplicaState {
    fn from(certificate: validator::CommitQC) -> Self {
        Self {
            view: certificate.message.view,
            phase: validator::Phase::Prepare,
            high_vote: certificate.message,
            high_qc: certificate,
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
            high_vote: read_required(&r.high_vote).context("high_vote")?,
            high_qc: read_required(&r.high_qc).context("high_qc")?,
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
            high_vote: Some(self.high_vote.build()),
            high_qc: Some(self.high_qc.build()),
            proposals: self.proposals.iter().map(|p| p.build()).collect(),
        }
    }
}


