use zksync_concurrency::time;

use crate::{
    attester::{self, AggregateSignature},
    validator::Genesis,
};

use super::{Signed, Signers};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default, PartialOrd)]
/// A batch number.
pub struct BatchNumber(pub u64);

impl BatchNumber {
    /// Increment the batch number.
    pub fn next_batch_number(&self) -> BatchNumber {
        BatchNumber(self.0.checked_add(1).unwrap_or(0))
    }
}

/// A message to send by attesters to the gossip network.
/// It contains the attester signature to sign the block batches to be sent to L1.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Batch {
    /// The number of the batch.
    pub number: BatchNumber,
    /// Time at which this message has been signed.
    pub timestamp: time::Utc,
    // TODO: add the hash of the L1 batch as a field
}

/// A certificate for a batch of L2 blocks to be sent to L1.
/// It contains the signatures of the attesters that signed the batch.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct BatchQC {
    /// The aggregate signature of the signed L1 batches.
    pub signature: AggregateSignature,
    /// The attesters that signed this message.
    pub signers: Signers,
    /// The message that was signed.
    pub message: Batch,
}

/// Error returned by `BatchQC::verify()` if the signature is invalid.
#[derive(thiserror::Error, Debug)]
pub enum BatchQCVerifyError {
    /// Bad signature.
    #[error("bad signature: {0:#}")]
    BadSignature(#[source] anyhow::Error),
    /// Not enough signers.
    #[error("not enough signers: got {got}, want {want}")]
    NotEnoughSigners {
        /// Got signers.
        got: u64,
        /// Want signers.
        want: u64,
    },
    /// Bad signer set.
    #[error("signers set doesn't match genesis")]
    BadSignersSet,
}

impl Batch {
    /// Checks if `self` is a newer version than `b`.
    pub fn is_newer(&self, b: &Self) -> bool {
        (&self.number, self.timestamp) > (&b.number, b.timestamp)
    }
}

impl BatchQC {
    /// Create a new empty instance for a given `Batch` message.
    pub fn new(message: Batch, genesis: &Genesis) -> Self {
        Self {
            message,
            signers: Signers::new(genesis.attesters.len()),
            signature: attester::AggregateSignature::default(),
        }
    }

    /// Add a attester's signature.
    /// Signature is assumed to be already verified.
    pub fn add(&mut self, msg: &Signed<Batch>, genesis: &Genesis) {
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

    /// Verifies the signature of the BatchQC.
    pub fn verify(&self, genesis: &Genesis) -> Result<(), BatchQCVerifyError> {
        use BatchQCVerifyError as Error;
        if self.signers.len() != genesis.attesters.len() {
            return Err(Error::BadSignersSet);
        }
        // Verify the signers' weight is enough.
        let weight = genesis.attesters.weight(&self.signers);
        let threshold = genesis.attesters.threshold();
        if weight < threshold {
            return Err(Error::NotEnoughSigners {
                got: weight,
                want: threshold,
            });
        }

        let messages_and_keys = genesis
            .attesters
            .iter_keys()
            .enumerate()
            .filter(|(i, _)| self.signers.0[*i])
            .map(|(_, pk)| (self.message.clone(), pk));

        self.signature
            .verify_messages(messages_and_keys)
            .map_err(Error::BadSignature)
    }
}
