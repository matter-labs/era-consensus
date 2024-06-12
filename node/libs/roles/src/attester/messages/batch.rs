use std::collections::BTreeMap;

use super::Signed;
use crate::{attester, validator::Genesis};
use anyhow::{ensure, Context as _};
use zksync_consensus_utils::enum_util::Variant;

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
    /// The signatures of the signed L1 batches from all the attesters who signed it.
    pub signatures: BTreeMap<attester::PublicKey, attester::Signature>,
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
    pub fn new(message: Batch) -> anyhow::Result<Self> {
        Ok(Self {
            message,
            signatures: BTreeMap::default(),
        })
    }

    /// Add a attester's signature.
    /// Signature is assumed to be already verified.
    pub fn add(&mut self, msg: &Signed<Batch>, genesis: &Genesis) -> anyhow::Result<()> {
        use BatchQCAddError as Error;

        let committee = genesis
            .attesters
            .as_ref()
            .context("no attester committee in genesis")?;

        ensure!(self.message == msg.msg, Error::InconsistentMessages);
        ensure!(!self.signatures.contains_key(&msg.key), Error::Exists);
        ensure!(
            committee.contains(&msg.key),
            Error::SignerNotInCommittee {
                signer: Box::new(msg.key.clone()),
            }
        );

        self.signatures.insert(msg.key.clone(), msg.sig.clone());

        Ok(())
    }

    /// Verifies the signature of the BatchQC.
    pub fn verify(&self, genesis: &Genesis) -> Result<(), BatchQCVerifyError> {
        use BatchQCVerifyError as Error;
        let attesters = genesis
            .attesters
            .as_ref()
            .ok_or(Error::AttestersNotInGenesis)?;

        // Verify that the signer's weight is sufficient.
        let weight = attesters.weight_of_keys(self.signatures.keys());
        let threshold = attesters.threshold();
        if weight < threshold {
            return Err(Error::NotEnoughSigners {
                got: weight,
                want: threshold,
            });
        }

        let hash = self.message.clone().insert().hash();

        for (pk, sig) in &self.signatures {
            sig.verify_hash(&hash, pk).map_err(Error::BadSignature)?;
        }

        Ok(())
    }
}
