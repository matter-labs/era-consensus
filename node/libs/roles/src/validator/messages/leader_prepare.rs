use super::{
    BlockHeader, BlockNumber, CommitQC, Genesis, Payload, ReplicaPrepare,
    ReplicaPrepareVerifyError, Signed, Signers, View,
};
use crate::validator;
use std::collections::{BTreeMap, HashMap};

/// A quorum certificate of replica Prepare messages. Since not all Prepare messages are
/// identical (they have different high blocks and high QCs), we need to keep the high blocks
/// and high QCs in a map. We can still aggregate the signatures though.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrepareQC {
    /// View of this QC.
    pub view: View,
    /// Map from replica Prepare messages to the validators that signed them.
    pub map: BTreeMap<ReplicaPrepare, Signers>,
    /// Aggregate signature of the replica Prepare messages.
    pub signature: validator::AggregateSignature,
}

/// Error returned by `PrepareQC::verify()`.
#[derive(thiserror::Error, Debug)]
pub enum PrepareQCVerifyError {
    /// Inconsistent views.
    #[error("inconsistent views of signed messages")]
    InconsistentViews,
    /// Invalid message.
    #[error("msg[{0}]: {1:#}")]
    InvalidMessage(usize, ReplicaPrepareVerifyError),
    /// Bad message format.
    #[error(transparent)]
    BadFormat(anyhow::Error),
    /// Weight not reached.
    #[error("Signers have not reached threshold weight: got {got}, want {want}")]
    NotEnoughSigners {
        /// Got weight.
        got: u64,
        /// Want weight.
        want: u64,
    },
    /// Bad signature.
    #[error("bad signature: {0:#}")]
    BadSignature(#[source] anyhow::Error),
}

impl PrepareQC {
    /// Create a new empty instance for a given `ReplicaCommit` message and a validator set size.
    pub fn new(view: View) -> Self {
        Self {
            view,
            map: BTreeMap::new(),
            signature: validator::AggregateSignature::default(),
        }
    }

    /// Get the highest block voted and check if there's a quorum of votes for it. To have a quorum
    /// in this situation, we require 2*f+1 votes, where f is the maximum number of faulty replicas.
    pub fn high_vote(&self, genesis: &Genesis) -> Option<BlockHeader> {
        let mut count: HashMap<_, u64> = HashMap::new();
        for (msg, signers) in &self.map {
            if let Some(v) = &msg.high_vote {
                *count.entry(v.proposal).or_default() += genesis.committee.weight(signers);
            }
        }
        // We only take one value from the iterator because there can only be at most one block with a quorum of 2f+1 votes.
        let min = 2 * genesis.committee.max_faulty_weight() + 1;
        count.into_iter().find(|x| x.1 >= min).map(|x| x.0)
    }

    /// Get the highest CommitQC.
    pub fn high_qc(&self) -> Option<&CommitQC> {
        self.map
            .keys()
            .filter_map(|m| m.high_qc.as_ref())
            .max_by_key(|qc| qc.view().number)
    }

    /// Add a validator's signed message.
    /// Message is assumed to be already verified.
    // TODO: check if there is already a message from that validator.
    // TODO: verify the message inside instead.
    pub fn add(&mut self, msg: &Signed<ReplicaPrepare>, genesis: &Genesis) {
        if msg.msg.view != self.view {
            return;
        }
        let Some(i) = genesis.committee.index(&msg.key) else {
            return;
        };
        let e = self
            .map
            .entry(msg.msg.clone())
            .or_insert_with(|| Signers::new(genesis.committee.len()));
        if e.0[i] {
            return;
        };
        e.0.set(i, true);
        self.signature.add(&msg.sig);
    }

    /// Verifies the integrity of the PrepareQC.
    pub fn verify(&self, genesis: &Genesis) -> Result<(), PrepareQCVerifyError> {
        use PrepareQCVerifyError as Error;
        let mut sum = Signers::new(genesis.committee.len());

        // Check the ReplicaPrepare messages.
        for (i, (msg, signers)) in self.map.iter().enumerate() {
            if msg.view != self.view {
                return Err(Error::InconsistentViews);
            }
            if signers.len() != sum.len() {
                return Err(Error::BadFormat(anyhow::format_err!(
                    "msg[{i}].signers has wrong length"
                )));
            }
            if signers.is_empty() {
                return Err(Error::BadFormat(anyhow::format_err!(
                    "msg[{i}] has no signers assigned"
                )));
            }
            if !(&sum & signers).is_empty() {
                return Err(Error::BadFormat(anyhow::format_err!(
                    "overlapping signature sets for different messages"
                )));
            }
            msg.verify(genesis)
                .map_err(|err| Error::InvalidMessage(i, err))?;
            sum |= signers;
        }

        // Verify the signers' weight is enough.
        let weight = genesis.committee.weight(&sum);
        let threshold = genesis.committee.threshold();
        if weight < threshold {
            return Err(Error::NotEnoughSigners {
                got: weight,
                want: threshold,
            });
        }
        // Now we can verify the signature.
        let messages_and_keys = self.map.clone().into_iter().flat_map(|(msg, signers)| {
            genesis
                .committee
                .keys()
                .enumerate()
                .filter(|(i, _)| signers.0[*i])
                .map(|(_, pk)| (msg.clone(), pk))
                .collect::<Vec<_>>()
        });
        // TODO(gprusak): This reaggregating is suboptimal.
        self.signature
            .verify_messages(messages_and_keys)
            .map_err(Error::BadSignature)
    }

    /// Calculates the weight of current PrepareQC signing validators
    pub fn weight(&self, committee: &validator::Committee) -> u64 {
        self.map
            .values()
            .map(|signers| committee.weight(signers))
            .sum()
    }
}

/// A Prepare message from a leader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaderPrepare {
    /// The header of the block that the leader is proposing.
    pub proposal: BlockHeader,
    /// Payload of the block that the leader is proposing.
    /// `None` iff this is a reproposal.
    pub proposal_payload: Option<Payload>,
    /// The PrepareQC that justifies this proposal from the leader.
    pub justification: PrepareQC,
}

/// Error returned by `LeaderPrepare::verify()`.
#[derive(thiserror::Error, Debug)]
pub enum LeaderPrepareVerifyError {
    /// Justification
    #[error("justification: {0:#}")]
    Justification(PrepareQCVerifyError),
    /// Bad block number.
    #[error("bad block number: got {got:?}, want {want:?}")]
    BadBlockNumber {
        /// Correct proposal number.
        want: BlockNumber,
        /// Received proposal number.
        got: BlockNumber,
    },
    /// New block proposal when the previous proposal was not finalized.
    #[error("new block proposal when the previous proposal was not finalized")]
    ProposalWhenPreviousNotFinalized,
    /// Mismatched payload.
    #[error("block proposal with mismatched payload")]
    ProposalMismatchedPayload,
    /// Re-proposal without quorum.
    #[error("block re-proposal without quorum for the re-proposal")]
    ReproposalWithoutQuorum,
    /// Re-proposal when the previous proposal was finalized.
    #[error("block re-proposal when the previous proposal was finalized")]
    ReproposalWhenFinalized,
    /// Reproposed a bad block.
    #[error("Reproposed a bad block")]
    ReproposalBadBlock,
}

impl LeaderPrepare {
    /// View of the message.
    pub fn view(&self) -> &View {
        &self.justification.view
    }

    /// Verifies LeaderPrepare.
    pub fn verify(&self, genesis: &Genesis) -> Result<(), LeaderPrepareVerifyError> {
        use LeaderPrepareVerifyError as Error;
        self.justification
            .verify(genesis)
            .map_err(Error::Justification)?;
        let high_vote = self.justification.high_vote(genesis);
        let high_qc = self.justification.high_qc();

        // Check that the proposal is valid.
        match &self.proposal_payload {
            // The leader proposed a new block.
            Some(payload) => {
                // Check that payload matches the header
                if self.proposal.payload != payload.hash() {
                    return Err(Error::ProposalMismatchedPayload);
                }
                // Check that we finalized the previous block.
                if high_vote.is_some()
                    && high_vote.as_ref() != high_qc.map(|qc| &qc.message.proposal)
                {
                    return Err(Error::ProposalWhenPreviousNotFinalized);
                }
                let want_number = match high_qc {
                    Some(qc) => qc.header().number.next(),
                    None => genesis.first_block,
                };
                if self.proposal.number != want_number {
                    return Err(Error::BadBlockNumber {
                        got: self.proposal.number,
                        want: want_number,
                    });
                }
            }
            None => {
                let Some(high_vote) = &high_vote else {
                    return Err(Error::ReproposalWithoutQuorum);
                };
                if let Some(high_qc) = &high_qc {
                    if high_vote.number == high_qc.header().number {
                        return Err(Error::ReproposalWhenFinalized);
                    }
                }
                if high_vote != &self.proposal {
                    return Err(Error::ReproposalBadBlock);
                }
            }
        }
        Ok(())
    }
}
