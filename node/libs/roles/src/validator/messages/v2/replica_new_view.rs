use anyhow::Context as _;
use zksync_protobuf::{read_required, ProtoFmt};

use super::{ProposalJustification, ProposalJustificationVerifyError, View};
use crate::{
    proto::validator as proto,
    validator::{self, EpochNumber, GenesisHash},
};

/// A new view message from a replica.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplicaNewView {
    /// What attests to the validity of this view change.
    pub justification: ProposalJustification,
}

impl ReplicaNewView {
    /// View of the message.
    pub fn view(&self) -> View {
        self.justification.view()
    }

    /// Verifies ReplicaNewView.
    pub fn verify(
        &self,
        genesis: GenesisHash,
        epoch: EpochNumber,
        validators_schedule: &validator::Schedule,
    ) -> Result<(), ReplicaNewViewVerifyError> {
        // Check that the justification is valid.
        self.justification
            .verify(genesis, epoch, validators_schedule)
            .map_err(ReplicaNewViewVerifyError::Justification)?;

        Ok(())
    }
}

impl ProtoFmt for ReplicaNewView {
    type Proto = proto::ReplicaNewViewV2;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            justification: read_required(&r.justification).context("justification")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            justification: Some(self.justification.build()),
        }
    }
}

/// Error returned by `ReplicaNewView::verify()`.
#[derive(thiserror::Error, Debug)]
pub enum ReplicaNewViewVerifyError {
    /// Invalid Justification.
    #[error("justification: {0:#}")]
    Justification(ProposalJustificationVerifyError),
}
