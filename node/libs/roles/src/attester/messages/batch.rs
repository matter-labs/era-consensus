use super::{Signed, Signers};
use crate::{attester, validator::Genesis, validator::Payload};
use anyhow::{ensure, Context as _};

/// A batch of L2 blocks used for the peers to fetch and keep in sync.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SyncBatch {
    /// The number of the batch.
    pub number: BatchNumber,
    /// The payloads of the blocks the batch contains.
    pub payloads: Vec<Payload>,
    /// The proof of the batch.
    pub proof: Vec<u8>,
}

// impl SyncBatch {
//     /// Create a new instance of `SyncBatch` with the given number, payloads, and proof.
//     pub fn new(number: BatchNumber, payloads: Vec<Payload>, proof: Vec<u8>) -> Self {
//         Self {
//             number,
//             payloads,
//             proof,
//         }
//     }
// }

impl PartialOrd for SyncBatch {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.number.partial_cmp(&other.number)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
/// A batch number.
pub struct BatchNumber(pub u64);

impl BatchNumber {
    /// Increment the batch number.
    pub fn next(&self) -> BatchNumber {
        BatchNumber(self.0.checked_add(1).unwrap())
    }

    /// Returns the previous batch number.
    pub fn prev(self) -> Option<Self> {
        Some(Self(self.0.checked_sub(1)?))
    }
}

impl std::fmt::Display for BatchNumber {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, formatter)
    }
}

impl std::ops::Add<u64> for BatchNumber {
    type Output = BatchNumber;
    fn add(self, n: u64) -> Self {
        Self(self.0.checked_add(n).unwrap())
    }
}

/// A message containing information about a batch of blocks.
/// It is signed by the attesters and then propagated through the gossip network.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd)]
pub struct Batch {
    /// Header of the batch.
    pub number: BatchNumber,
    // TODO: Hash of the batch.
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
    /// Header of the certified Batch.
    pub fn number(&self) -> &BatchNumber {
        &self.message.number
    }

    /// Create a new empty instance for a given `Batch` message.
    pub fn new(message: Batch, genesis: &Genesis) -> anyhow::Result<Self> {
        Ok(Self {
            message,
            signers: Signers::new(genesis.attesters.as_ref().context("attesters")?.len()),
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
            .expect("attesters set is empty in genesis") // This case should never happen
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
            .expect("attesters set is empty in genesis"); // This case should never happen
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
        // The enumeration here goes by order of public key, which assumes
        // that the aggregate signature does not care about ordering.
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
