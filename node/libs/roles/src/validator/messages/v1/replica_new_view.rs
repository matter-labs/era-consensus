use crate::validator::Genesis;

use super::{ProposalJustification, ProposalJustificationVerifyError, View};

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
    pub fn verify(&self, genesis: &Genesis) -> Result<(), ReplicaNewViewVerifyError> {
        // Check that the justification is valid.
        self.justification
            .verify(genesis)
            .map_err(ReplicaNewViewVerifyError::Justification)?;

        Ok(())
    }
}

/// Error returned by `ReplicaNewView::verify()`.
#[derive(thiserror::Error, Debug)]
pub enum ReplicaNewViewVerifyError {
    /// Invalid Justification.
    #[error("justification: {0:#}")]
    Justification(ProposalJustificationVerifyError),
}
