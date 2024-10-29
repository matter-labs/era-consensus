use super::{
    BlockNumber, CommitQC, CommitQCVerifyError, Genesis, Payload, PayloadHash, TimeoutQC,
    TimeoutQCVerifyError, View,
};

/// A proposal message from the leader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaderProposal {
    /// Payload of the block that the leader is proposing.
    /// `None` iff this is a reproposal.
    pub proposal_payload: Option<Payload>,
    /// What attests to the validity of this proposal.
    pub justification: ProposalJustification,
}

impl LeaderProposal {
    /// View of the message.
    pub fn view(&self) -> View {
        self.justification.view()
    }

    /// Verifies LeaderProposal.
    pub fn verify(&self, genesis: &Genesis) -> Result<(), LeaderProposalVerifyError> {
        // Check that the justification is valid.
        self.justification
            .verify(genesis)
            .map_err(LeaderProposalVerifyError::Justification)
    }
}

/// Error returned by `LeaderProposal::verify()`.
#[derive(thiserror::Error, Debug)]
pub enum LeaderProposalVerifyError {
    /// Invalid Justification.
    #[error("Invalid justification: {0:#}")]
    Justification(ProposalJustificationVerifyError),
}

/// Justification for a proposal. This is either a Commit QC or a Timeout QC.
/// The first proposal, for view 0, will always be a timeout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProposalJustification {
    /// This proposal is being proposed after a view where we finalized a block.
    /// A commit QC is just a collection of commit votes (with at least
    /// QUORUM_WEIGHT) for the previous view. Note that the commit votes MUST
    /// be identical.
    Commit(CommitQC),
    /// This proposal is being proposed after a view where we timed out.
    /// A timeout QC is just a collection of timeout votes (with at least
    /// QUORUM_WEIGHT) for the previous view. Unlike with the Commit QC,
    /// timeout votes don't need to be identical.
    /// The first proposal, for view 0, will always be a timeout.
    Timeout(TimeoutQC),
}

impl ProposalJustification {
    /// View of the justification.
    pub fn view(&self) -> View {
        match self {
            ProposalJustification::Commit(qc) => qc.view().next(),
            ProposalJustification::Timeout(qc) => qc.view.next(),
        }
    }

    /// Verifies the justification.
    pub fn verify(&self, genesis: &Genesis) -> Result<(), ProposalJustificationVerifyError> {
        match self {
            ProposalJustification::Commit(qc) => qc
                .verify(genesis)
                .map_err(ProposalJustificationVerifyError::Commit),
            ProposalJustification::Timeout(qc) => qc
                .verify(genesis)
                .map_err(ProposalJustificationVerifyError::Timeout),
        }
    }

    /// This returns the BlockNumber that is implied by this justification.
    /// If the justification requires a block reproposal, it also returns
    /// the PayloadHash that must be reproposed.
    pub fn get_implied_block(&self, genesis: &Genesis) -> (BlockNumber, Option<PayloadHash>) {
        match self {
            ProposalJustification::Commit(qc) => {
                // The previous proposal was finalized, so we can propose a new block.
                (qc.header().number.next(), None)
            }
            ProposalJustification::Timeout(qc) => {
                // Get the high vote of the timeout QC, if it exists. We check if there are
                // timeout votes with at least an added weight of SUBQUORUM_WEIGHT,
                // that have a high vote field for the same block. A QC can have
                // 0, 1 or 2 such blocks.
                // If there's only 1 such block, then we say the QC has a high vote.
                // If there are 0 or 2 such blocks, we say the QC has no high vote.
                let high_vote = qc.high_vote(genesis);

                // Get the high commit QC of the timeout QC. We compare the high QC field of
                // all timeout votes in the QC, and get the highest one, if it exists.
                // The high QC always exists, unless no block has been finalized yet in the chain.
                let high_qc = qc.high_qc();

                // If there was a high vote in the timeout QC, and either there was no high QC
                // in the timeout QC, or the high vote is for a higher block than the high QC,
                // then we need to repropose the high vote.
                #[allow(clippy::unnecessary_unwrap)] // using a match would be more verbose
                if high_vote.is_some()
                    && (high_qc.is_none()
                        || high_vote.unwrap().number > high_qc.unwrap().header().number)
                {
                    // There was some proposal last view that might have been finalized.
                    // We need to repropose it.
                    (high_vote.unwrap().number, Some(high_vote.unwrap().payload))
                } else {
                    // Either the previous proposal was finalized or we know for certain
                    // that it couldn't have been finalized (because there is no high vote).
                    // Either way, we can propose a new block.

                    // If there is no high QC, then we must be at the start of the chain.
                    let block_number = match high_qc {
                        Some(qc) => qc.header().number.next(),
                        None => genesis.first_block,
                    };

                    (block_number, None)
                }
            }
        }
    }
}

/// Error returned by `ProposalJustification::verify()`.
#[derive(thiserror::Error, Debug)]
pub enum ProposalJustificationVerifyError {
    /// Invalid timeout QC.
    #[error("Invalid timeout QC: {0:#}")]
    Timeout(TimeoutQCVerifyError),
    /// Invalid commit QC.
    #[error("Invalid commit QC: {0:#}")]
    Commit(CommitQCVerifyError),
}