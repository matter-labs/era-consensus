use anyhow::Context as _;
use zksync_consensus_crypto::ByteFmt;
use zksync_protobuf::{read_required, required, ProtoFmt};

use super::{CommitQC, CommitQCVerifyError};
use crate::{
    proto::validator as proto,
    validator::{self, BlockNumber, EpochNumber, GenesisHash, Payload, PayloadHash},
};

/// A block header.
/// WARNING: any change to this struct may invalidate preexisting signatures. See `TimeoutQC` docs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockHeader {
    /// Number of the block.
    pub number: BlockNumber,
    /// Payload of the block.
    pub payload: PayloadHash,
}

impl ProtoFmt for BlockHeader {
    type Proto = proto::BlockHeaderV2;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            number: BlockNumber(*required(&r.number).context("number")?),
            payload: read_required(&r.payload).context("payload")?,
        })
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            number: Some(self.number.0),
            payload: Some(self.payload.build()),
        }
    }
}

/// A block that has been finalized by the consensus protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalBlock {
    /// Payload of the block. Should match `header.payload` hash.
    pub payload: Payload,
    /// Justification for the block. What guarantees that the block is final.
    pub justification: CommitQC,
}

impl FinalBlock {
    /// Creates a new finalized block.
    pub fn new(payload: Payload, justification: CommitQC) -> Self {
        assert_eq!(justification.header().payload, payload.hash());

        Self {
            payload,
            justification,
        }
    }

    /// Header of the block.
    pub fn header(&self) -> &BlockHeader {
        &self.justification.message.proposal
    }

    /// Number of the block.
    pub fn number(&self) -> BlockNumber {
        self.header().number
    }

    /// Verifies internal consistency of this block.
    pub fn verify(
        &self,
        genesis: GenesisHash,
        epoch: EpochNumber,
        validators_schedule: &validator::Schedule,
    ) -> Result<(), BlockValidationError> {
        let payload_hash = self.payload.hash();
        if payload_hash != self.header().payload {
            return Err(BlockValidationError::HashMismatch {
                header_hash: self.header().payload,
                payload_hash,
            });
        }
        self.justification
            .verify(genesis, epoch, validators_schedule)
            .map_err(BlockValidationError::Justification)
    }
}

impl ByteFmt for FinalBlock {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        zksync_protobuf::decode(bytes)
    }

    fn encode(&self) -> Vec<u8> {
        zksync_protobuf::encode(self)
    }
}

impl ProtoFmt for FinalBlock {
    type Proto = proto::FinalBlockV2;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            payload: Payload(required(&r.payload).context("payload")?.clone()),
            justification: read_required(&r.justification).context("justification")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            payload: Some(self.payload.0.clone()),
            justification: Some(self.justification.build()),
        }
    }
}

/// Errors that can occur validating a `FinalBlock` received from a node.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BlockValidationError {
    /// Block payload doesn't match the block header.
    #[error(
        "block payload doesn't match the block header (hash in header: {header_hash:?}, payload \
         hash: {payload_hash:?})"
    )]
    HashMismatch {
        /// Payload hash in block header.
        header_hash: PayloadHash,
        /// Hash of the payload.
        payload_hash: PayloadHash,
    },
    /// Failed verifying quorum certificate.
    #[error("failed verifying quorum certificate: {0:#?}")]
    Justification(#[source] CommitQCVerifyError),
}
