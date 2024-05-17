use crate::{attester, validator::Genesis};

use super::{Signed, Signers};
use anyhow::{ensure, Context as _};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default, PartialOrd)]
/// A batch number.
pub struct BatchNumber(pub u64);

impl BatchNumber {
    /// Increment the batch number.
    pub fn next(&self) -> BatchNumber {
        BatchNumber(self.0.checked_add(1).unwrap())
    }
}

/// A message containing information about a batch of blocks.
/// It is signed by the attesters and then propagated through the gossip network.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd)]
pub struct Batch {
    /// The number of the batch.
    pub number: BatchNumber,
    // TODO: add the hash of the L1 batch as a field
}

/// A certificate for a batch of L2 blocks to be sent to L1.
/// It contains the signatures of the attesters that signed the batch.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct BatchQC {
    /// The aggregate signature of the signed L1 batches.
    pub signature: attester::AggregateSignature,
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
    /// No attester committee in genesis.
    #[error("No attester committee in genesis")]
    AttestersNotInGenesis,
}

/// Error returned by `BatchQC::add()` if the signature is invalid.
#[derive(thiserror::Error, Debug)]
pub enum BatchQCAddError {
    /// Inconsistent messages.
    #[error("Trying to add signature for a different message")]
    InconsistentMessages,
    /// Signer not present in the committee.
    #[error("Signer not in committee: {signer:?}")]
    SignerNotInCommittee {
        /// Signer of the message.
        signer: Box<attester::PublicKey>,
    },
    /// Message already present in BatchQC.
    #[error("Message already signed for BatchQC")]
    Exists,
}

impl BatchQC {
    /// Create a new empty instance for a given `Batch` message.
    pub fn new(message: Batch, genesis: &Genesis) -> anyhow::Result<Self> {
        Ok(Self {
            message,
            signers: Signers::new(
                genesis
                    .attesters
                    .as_ref()
                    .context("no attester committee in genesis")?
                    .len(),
            ),
            signature: attester::AggregateSignature::default(),
        })
    }

    /// Add a attester's signature.
    /// Signature is assumed to be already verified.
    pub fn add(&mut self, msg: &Signed<Batch>, genesis: &Genesis) -> anyhow::Result<()> {
        use BatchQCAddError as Error;
        ensure!(self.message == msg.msg, Error::InconsistentMessages);
        let i = genesis
            .attesters
            .as_ref()
            .context("no attester committee in genesis")?
            .index(&msg.key)
            .ok_or(Error::SignerNotInCommittee {
                signer: Box::new(msg.key.clone()),
            })?;
        ensure!(!self.signers.0[i], Error::Exists);
        self.signers.0.set(i, true);
        self.signature.add(&msg.sig);
        Ok(())
    }

    /// Verifies the signature of the BatchQC.
    pub fn verify(&self, genesis: &Genesis) -> Result<(), BatchQCVerifyError> {
        use BatchQCVerifyError as Error;
        let attesters = genesis
            .attesters
            .as_ref()
            .ok_or(Error::AttestersNotInGenesis)?;
        if self.signers.len() != attesters.len() {
            return Err(Error::BadSignersSet);
        }
        // Verify that the signer's weight is sufficient.
        let weight = attesters.weight(&self.signers);
        let threshold = attesters.threshold();
        if weight < threshold {
            return Err(Error::NotEnoughSigners {
                got: weight,
                want: threshold,
            });
        }

        let messages_and_keys = attesters
            .keys()
            .enumerate()
            .filter(|(i, _)| self.signers.0[*i])
            .map(|(_, pk)| (self.message.clone(), pk));

        self.signature
            .verify_messages(messages_and_keys)
            .map_err(Error::BadSignature)
    }
}
