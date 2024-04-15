use crate::validator::{self};

use super::{Genesis, Signed};

/// A message to send by validators to the gossip network.
/// It contains the validators signature to sign the block batches to be sent to L1.
#[derive(Debug, Clone, Eq, PartialEq, Default)]
pub struct L1BatchMsg;

/// A certificate for a batch of L2 blocks to be sent to L1.
/// It contains the signatures of the validators that signed the batch.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct L1BatchQC {
    /// The aggregate signature of the signed L1 batches.
    pub signature: validator::AggregateSignature,
    /// The validators that signed this message.
    pub signers: validator::Signers,
    /// The message that was signed.
    pub message: L1BatchMsg,
}

/// Error returned by `L1BatchQC::verify()` if the signature is invalid.
#[derive(thiserror::Error, Debug)]
pub enum L1BatchQCVerifyError {
    /// Bad signature.
    #[error("bad signature: {0:#}")]
    BadSignature(#[source] anyhow::Error),
}

impl L1BatchQC {
    /// Create a new empty instance for a given `ReplicaCommit` message and a validator set size.
    pub fn new(validators: usize) -> Self {
        Self {
            signature: validator::AggregateSignature::default(),
            signers: validator::Signers::new(validators),
            message: L1BatchMsg,
        }
    }

    /// Add a attester's signature.
    /// Signature is assumed to be already verified.
    pub fn add(&mut self, msg: &Signed<L1BatchMsg>, genesis: &Genesis) {
        if self.message != msg.msg {
            return;
        };
        let Some(i) = genesis.validators.index(&msg.key) else {
            return;
        };
        if self.signers.0[i] {
            return;
        };
        self.signers.0.set(i, true);
        self.signature.add(&msg.sig);
    }

    /// Verifies the signature of the CommitQC.
    pub fn verify(&self, genesis: &Genesis) -> Result<(), L1BatchQCVerifyError> {
        // Now we can verify the signature.
        let messages_and_keys = genesis
            .validators
            .iter()
            .enumerate()
            .filter(|(i, _)| self.signers.0[*i])
            .map(|(_, pk)| (self.message.clone(), pk));

        self.signature
            .verify_messages(messages_and_keys)
            .map_err(L1BatchQCVerifyError::BadSignature)
    }
}
