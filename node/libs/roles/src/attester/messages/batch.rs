use super::{Signed, Signers};
use crate::{attester, validator::Genesis};
use anyhow::{ensure, Context as _};
use zksync_consensus_crypto::{keccak256::Keccak256, ByteFmt, Text, TextFmt};

/// Payload of the batch. Consensus algorithm does not interpret the payload
/// (except for imposing a size limit for the payload). Proposing a payload
/// for a new batch and interpreting the payload of the finalized batches
/// should be implemented for the specific application of the consensus algorithm.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Payload(pub Vec<u8>);

impl Payload {
    /// Hash of the payload.
    pub fn hash(&self) -> PayloadHash {
        PayloadHash(Keccak256::new(&self.0))
    }
}

/// Errors that can occur validating a `FinalBatch` received from a node.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BatchValidationError {
    /// Batch payload doesn't match the batch header.
    #[error(
        "batch payload doesn't match the batch header (hash in header: {header_hash:?}, \
             payload hash: {payload_hash:?})"
    )]
    HashMismatch {
        /// Payload hash in batch header.
        header_hash: PayloadHash,
        /// Hash of the payload.
        payload_hash: PayloadHash,
    },
    /// Failed verifying quorum certificate.
    #[error("failed verifying quorum certificate: {0:#?}")]
    Justification(#[source] BatchQCVerifyError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

/// Hash of the Payload.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PayloadHash(pub(crate) Keccak256);

impl TextFmt for PayloadHash {
    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("payload:keccak256:")?.decode_hex().map(Self)
    }

    fn encode(&self) -> String {
        format!(
            "payload:keccak256:{}",
            hex::encode(ByteFmt::encode(&self.0))
        )
    }
}

impl std::fmt::Debug for PayloadHash {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt.write_str(&TextFmt::encode(self))
    }
}

/// A batch header.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BatchHeader {
    /// Number of the batch.
    pub number: BatchNumber,
    /// Payload of the batch.
    pub payload: PayloadHash,
}

/// A batch that has been finalized by the consensus protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalBatch {
    /// Payload of the batch. Should match `header.payload` hash.
    pub payload: Payload,
    /// Justification for the batch. What guarantees that the batch is final.
    pub justification: BatchQC,
}

impl FinalBatch {
    /// Creates a new finalized Batch.
    pub fn new(payload: Payload, justification: BatchQC) -> Self {
        assert_eq!(justification.header().payload, payload.hash());
        Self {
            payload,
            justification,
        }
    }

    /// Header of the batch.
    pub fn header(&self) -> &BatchHeader {
        &self.justification.message.proposal
    }

    /// Number of the batch.
    pub fn number(&self) -> BatchNumber {
        self.header().number
    }

    /// Verifies internal consistency of this batch.
    pub fn verify(&self, genesis: &Genesis) -> Result<(), BatchValidationError> {
        let payload_hash = self.payload.hash();
        if payload_hash != self.header().payload {
            return Err(BatchValidationError::HashMismatch {
                header_hash: self.header().payload,
                payload_hash,
            });
        }
        self.justification
            .verify(genesis)
            .map_err(BatchValidationError::Justification)
    }
}

/// A message containing information about a batch of blocks.
/// It is signed by the attesters and then propagated through the gossip network.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd)]
pub struct Batch {
    /// Header of the batch.
    pub proposal: BatchHeader,
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
    pub fn header(&self) -> &BatchHeader {
        &self.message.proposal
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
