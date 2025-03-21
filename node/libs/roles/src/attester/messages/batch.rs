use zksync_consensus_crypto::{keccak256::Keccak256, ByteFmt, Text, TextFmt};
use zksync_consensus_utils::enum_util::Variant;

use super::{GenesisHash, Signed};
use crate::attester;

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

/// Hash of the L1 batch.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BatchHash(pub Keccak256);

impl TextFmt for BatchHash {
    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("batch:keccak256:")?.decode_hex().map(Self)
    }

    fn encode(&self) -> String {
        format!("batch:keccak256:{}", hex::encode(ByteFmt::encode(&self.0)))
    }
}

impl std::fmt::Debug for BatchHash {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt.write_str(&TextFmt::encode(self))
    }
}

/// A message containing information about a batch of blocks.
/// It is signed by the attesters and then propagated through the gossip network.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd)]
pub struct Batch {
    /// Header of the batch.
    pub number: BatchNumber,
    /// Hash of the batch.
    pub hash: BatchHash,
    /// Hash of the genesis.
    ///
    /// This includes the chain ID and the current fork number, which prevents
    /// replay attacks from other chains where the same attesters might operate,
    /// or from earlier forks, which are created after a revert of L1 batches.
    pub genesis: GenesisHash,
}

/// A certificate for a batch of L2 blocks to be sent to L1.
/// It contains the signatures of the attesters that signed the batch.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct BatchQC {
    /// The signatures of the signed L1 batches from all the attesters who signed it.
    pub signatures: attester::MultiSig,
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
    #[error("signers not in committee")]
    BadSignersSet,
    /// Genesis mismatch.
    #[error("genesis mismatch")]
    GenesisMismatch,
}

impl BatchQC {
    /// Create a new empty instance for a given `Batch` message.
    pub fn new(message: Batch) -> Self {
        Self {
            message,
            signatures: attester::MultiSig::default(),
        }
    }

    /// Add a attester's signature.
    /// Signature is assumed to be already verified.
    pub fn add(
        &mut self,
        msg: &Signed<Batch>,
        committee: &attester::Committee,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(self.message == msg.msg, "inconsistent messages");
        anyhow::ensure!(
            !self.signatures.contains(&msg.key),
            "signature already present"
        );
        anyhow::ensure!(committee.contains(&msg.key), "not in committee");
        self.signatures.add(msg.key.clone(), msg.sig.clone());
        Ok(())
    }

    /// Verifies the the BatchQC.
    pub fn verify(
        &self,
        genesis: GenesisHash,
        committee: &attester::Committee,
    ) -> Result<(), BatchQCVerifyError> {
        use BatchQCVerifyError as Error;

        if self.message.genesis != genesis {
            return Err(Error::GenesisMismatch);
        }

        // Verify that all signers are attesters.
        for pk in self.signatures.keys() {
            if !committee.contains(pk) {
                return Err(Error::BadSignersSet);
            }
        }

        // Verify that the signer's weight is sufficient.
        let weight = committee.weight_of_keys(self.signatures.keys());
        let threshold = committee.threshold();
        if weight < threshold {
            return Err(Error::NotEnoughSigners {
                got: weight,
                want: threshold,
            });
        }

        self.signatures
            .verify_msg(&self.message.clone().insert())
            .map_err(Error::BadSignature)
    }
}
