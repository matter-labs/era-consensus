use crate::{
    attester::BatchAggregateSignature,
    validator::{self, Genesis, Signature},
};

use super::{Attesters, SignedBatchMsg};

/// A message to send by validators to the gossip network.
/// It contains the validators signature to sign the block batches to be sent to L1.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct L1BatchMsg();

/// A certificate for a batch of L2 blocks to be sent to L1.
/// It contains the signatures of the validators that signed the batch.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct L1BatchQC {
    /// The aggregate signature of the signed L1 batches.
    pub signature: BatchAggregateSignature,
    /// The validators that signed this message.
    pub attesters: Attesters,
    /// The message that was signed.
    pub message: L1BatchMsg,
}

#[derive(thiserror::Error, Debug)]
pub enum L1BatchQCVerifyError {
    /// Bad signature.
    #[error("bad signature: {0:#}")]
    BadSignature(#[source] anyhow::Error),
}

impl L1BatchQC {
    pub fn add(&mut self, signature: Signature, msg: &SignedBatchMsg, genesis: &Genesis) {
        if self.message != msg.msg {
            return;
        };
        let Some(i) = genesis.validators.index(&msg.key) else {
            return;
        };
        if self.attesters.0[i] {
            return;
        };
        self.attesters.0.set(i, true);
        self.signature.add(&msg.sig);
    }

    /// Verifies the signature of the CommitQC.
    pub fn verify(&self, genesis: &Genesis) -> Result<(), L1BatchQCVerifyError> {
        // Now we can verify the signature.
        let messages_and_keys = genesis
            .validators
            .iter()
            .enumerate()
            .filter(|(i, _)| self.attesters.0[*i])
            .map(|(_, pk)| (self.message.clone(), pk));

        self.signature
            .verify_messages(messages_and_keys)
            .map_err(L1BatchQCVerifyError::BadSignature)
    }
}
