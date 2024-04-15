use crate::{
    attester::{self, AggregateSignature},
    validator::Genesis,
};

use super::{SignedBatchMsg, Signers};

/// A message to send by validators to the gossip network.
/// It contains the validators signature to sign the block batches to be sent to L1.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct L1Batch;

/// A certificate for a batch of L2 blocks to be sent to L1.
/// It contains the signatures of the validators that signed the batch.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct L1BatchQC {
    /// The aggregate signature of the signed L1 batches.
    pub signature: AggregateSignature,
    /// The validators that signed this message.
    pub signers: Signers,
    /// The message that was signed.
    pub message: L1Batch,
}

/// Error returned by `L1BatchQC::verify()` if the signature is invalid.
#[derive(thiserror::Error, Debug)]
pub enum L1BatchQCVerifyError {
    /// Bad signature.
    #[error("bad signature: {0:#}")]
    BadSignature(#[source] anyhow::Error),
    /// Not enough signers.
    #[error("not enough signers: got {got}, want {want}")]
    NotEnoughSigners {
        /// Got signers.
        got: usize,
        /// Want signers.
        want: usize,
    },
}

impl L1BatchQC {
    /// Create a new empty instance for a given `ReplicaCommit` message and a validator set size.
    pub fn new(message: L1Batch, genesis: &Genesis) -> Self {
        Self {
            message,
            signers: Signers::new(genesis.attesters.len()),
            signature: attester::AggregateSignature::default(),
        }
    }

    /// Add a attester's signature.
    /// Signature is assumed to be already verified.
    pub fn add(&mut self, msg: &SignedBatchMsg<L1Batch>, genesis: &Genesis) {
        if self.message != msg.msg {
            return;
        };
        let Some(i) = genesis.attesters.index(&msg.key) else {
            return;
        };
        if self.signers.0[i] {
            return;
        };
        self.signers.0.set(i, true);
        self.signature.add(&msg.sig);
    }

    /// Verifies the signature of the L1BatchQC.
    pub fn verify(&self, genesis: &Genesis) -> Result<(), L1BatchQCVerifyError> {
        // Verify that we have enough signers.
        let num_signers = self.signers.count();
        let threshold = genesis.attesters.threshold();
        if num_signers < threshold {
            return Err(L1BatchQCVerifyError::NotEnoughSigners {
                got: num_signers,
                want: threshold,
            });
        }

        let messages_and_keys = genesis
            .attesters
            .iter()
            .enumerate()
            .filter(|(i, _)| self.signers.0[*i])
            .map(|(_, pk)| (self.message.clone(), pk));

        self.signature
            .verify_messages(messages_and_keys)
            .map_err(L1BatchQCVerifyError::BadSignature)
    }
}