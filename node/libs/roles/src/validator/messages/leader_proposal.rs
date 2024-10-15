use super::{
    BlockHeader, BlockNumber, CommitQC, CommitQCVerifyError, Genesis, Payload, PayloadHash,
    TimeoutQC, TimeoutQCVerifyError, View,
};

/// A Proposal message from the leader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaderProposal {
    /// The header of the block that the leader is proposing.
    pub proposal: BlockHeader,
    /// Payload of the block that the leader is proposing.
    /// `None` iff this is a reproposal.
    pub proposal_payload: Option<Payload>,
    // What attests to the validity of this proposal.
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
            .map_err(LeaderProposalVerifyError::Justification)?;

        // Get the implied block number and payload hash and check it against the proposal.
        let (implied_block_number, implied_payload) = self.justification.get_implied_block(genesis);

        if self.proposal.number != implied_block_number {
            return Err(LeaderProposalVerifyError::BadBlockNumber {
                got: self.proposal.number,
                want: implied_block_number,
            });
        }

        if let Some(payload_hash) = implied_payload {
            if self.proposal.payload != payload_hash {
                return Err(LeaderProposalVerifyError::BadPayloadHash {
                    got: self.proposal.payload,
                    want: payload_hash,
                });
            }
        }

        // Check if we are correctly proposing a new block or re-proposing an old one.
        if implied_payload.is_none() && self.proposal_payload.is_none() {
            return Err(LeaderProposalVerifyError::ReproposalWhenPreviousFinalized);
        }

        if implied_payload.is_some() && self.proposal_payload.is_some() {
            return Err(LeaderProposalVerifyError::NewProposalWhenPreviousNotFinalized);
        }

        // Check that the payload matches the header.
        if let Some(payload) = &self.proposal_payload {
            if payload.hash() != self.proposal.payload {
                return Err(LeaderProposalVerifyError::MismatchedPayload {
                    header: self.proposal.payload,
                    payload: payload.hash(),
                });
            }
        }

        Ok(())
    }
}

/// Error returned by `LeaderProposal::verify()`.
#[derive(thiserror::Error, Debug)]
pub enum LeaderProposalVerifyError {
    /// Invalid Justification.
    #[error("justification: {0:#}")]
    Justification(ProposalJustificationVerifyError),
    /// Bad block number.
    #[error("bad block number: got {got:?}, want {want:?}")]
    BadBlockNumber {
        /// Received proposal number.
        got: BlockNumber,
        /// Correct proposal number.
        want: BlockNumber,
    },
    /// Bad payload hash on reproposal.
    #[error("bad payload hash on reproposal: got {got:?}, want {want:?}")]
    BadPayloadHash {
        /// Received payload hash.
        got: PayloadHash,
        /// Correct payload hash.
        want: PayloadHash,
    },
    /// New block proposal when the previous proposal was not finalized.
    #[error("new block proposal when the previous proposal was not finalized")]
    NewProposalWhenPreviousNotFinalized,
    /// Re-proposal when the previous proposal was finalized.
    #[error("block re-proposal when the previous proposal was finalized")]
    ReproposalWhenPreviousFinalized,
    /// Mismatched payload.
    #[error("block proposal with mismatched payload: header {header:?}, payload {payload:?}")]
    MismatchedPayload {
        /// Payload hash on block header.
        header: PayloadHash,
        /// Correct payload hash.
        payload: PayloadHash,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProposalJustification {
    // This proposal is being proposed after a view where we finalized a block.
    // A commit QC is just a collection of commit votes (with at least
    // QUORUM_WEIGHT) for the previous view. Note that the commit votes MUST
    // be identical.
    Commit(CommitQC),
    // This proposal is being proposed after a view where we timed out.
    // A timeout QC is just a collection of timeout votes (with at least
    // QUORUM_WEIGHT) for the previous view. Unlike with the Commit QC,
    // timeout votes don't need to be identical.
    // The first proposal, for view 0, will always be a timeout.
    Timeout(TimeoutQC),
}

impl ProposalJustification {
    pub fn view(&self) -> View {
        match self {
            ProposalJustification::Commit(qc) => qc.view().next(),
            ProposalJustification::Timeout(qc) => qc.view.next(),
        }
    }

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

    // This returns the BlockNumber that is implied by this justification.
    // If the justification requires a block reproposal, it also returns
    // the PayloadHash that must be reproposed.
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
                let high_qc = qc.high_qc();

                if high_vote.is_some()
                    && (high_qc.is_none()
                        || high_vote.unwrap().number > high_qc.unwrap().header().number)
                {
                    // There was some proposal last view that might have been finalized.
                    // We need to repropose it.
                    (high_vote.unwrap().number, Some(high_vote.unwrap().payload))
                } else {
                    // Either the previous proposal was finalized or we know for certain
                    // that it couldn't have been finalized. Either way, we can propose
                    // a new block.
                    let block_number = match high_qc {
                        Some(qc) => qc.header().number.next(),
                        None => BlockNumber(0),
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
    /// Invalid Timeout QC.
    #[error("timeout qc: {0:#}")]
    Timeout(TimeoutQCVerifyError),
    /// Invalid Commit QC.
    #[error("commit qc: {0:#}")]
    Commit(CommitQCVerifyError),
}
