//! Defines the schema of the database.

use anyhow::Context as _;
use concurrency::ctx;
use rocksdb::{Direction, IteratorMode};
use roles::validator::{self, BlockNumber};
use schema::{proto::storage as proto, read_required, required, ProtoFmt};
use std::{iter, ops};
use thiserror::Error;

/// Enum used to represent a key in the database. It also acts as a separator between different stores.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DatabaseKey {
    /// Key used to store the replica state.
    /// ReplicaState -> ReplicaState
    ReplicaState,
    /// Key used to store the finalized blocks.
    /// Block(BlockNumber) -> FinalBlock
    Block(BlockNumber),
}

impl DatabaseKey {
    /// Starting database key for blocks indexed by number. All other keys in the default column family
    /// are lower than this value.
    pub(crate) const BLOCKS_START_KEY: &'static [u8] = &u64::MIN.to_be_bytes();

    /// Iterator mode for the head block (i.e., a block with the greatest number).
    pub(crate) const BLOCK_HEAD_ITERATOR: IteratorMode<'static> =
        IteratorMode::From(&u64::MAX.to_be_bytes(), Direction::Reverse);

    /// Encodes this key for usage as a RocksDB key.
    ///
    /// # Implementation note
    ///
    /// This logic is maintainable only while the amount of non-block keys remains small.
    /// If more keys are added (especially if their number is not known statically), prefer using
    /// separate column families for them.
    pub(crate) fn encode_key(&self) -> Vec<u8> {
        match self {
            // Keys for non-block entries must be smaller than all block keys.
            Self::ReplicaState => vec![0],
            // Number encoding that monotonically increases with the number
            Self::Block(number) => number.0.to_be_bytes().to_vec(),
        }
    }

    /// Parses the specified bytes as a `Self::Block(_)` key.
    pub(crate) fn parse_block_key(raw_key: &[u8]) -> anyhow::Result<BlockNumber> {
        let raw_key = raw_key
            .try_into()
            .context("Invalid encoding for block key")?;
        Ok(BlockNumber(u64::from_be_bytes(raw_key)))
    }
}

/// A payload of a proposed block which is not known to be finalized yet.
/// Replicas have to persist such proposed payloads for liveness:
/// consensus may finalize a block without knowing a payload in case of reproposals.
/// Currently we do not store the BlockHeader, because it is always
///   available in the LeaderPrepare message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proposal {
    /// Number of a block for which this payload has been proposed.
    pub number: BlockNumber,
    /// Proposed payload.
    pub payload: validator::Payload,
}

/// The struct that contains the replica state to be persisted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplicaState {
    /// The current view number.
    pub view: validator::ViewNumber,
    /// The current phase.
    pub phase: validator::Phase,
    /// The highest block proposal that the replica has committed to.
    pub high_vote: validator::ReplicaCommit,
    /// The highest commit quorum certificate known to the replica.
    pub high_qc: validator::CommitQC,
    /// A cache of the received block proposals.
    pub proposals: Vec<Proposal>,
}

impl ProtoFmt for Proposal {
    type Proto = proto::Proposal;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            number: BlockNumber(*required(&r.number).context("number")?),
            payload: validator::Payload(required(&r.payload).context("payload")?.clone()),
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            number: Some(self.number.0),
            payload: Some(self.payload.0.clone()),
        }
    }
}

impl ProtoFmt for ReplicaState {
    type Proto = proto::ReplicaState;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            view: validator::ViewNumber(r.view.context("view_number")?),
            phase: read_required(&r.phase).context("phase")?,
            high_vote: read_required(&r.high_vote).context("high_vote")?,
            high_qc: read_required(&r.high_qc).context("high_qc")?,
            proposals: r
                .proposals
                .iter()
                .map(ProtoFmt::read)
                .collect::<Result<_, _>>()
                .context("proposals")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            view: Some(self.view.0),
            phase: Some(self.phase.build()),
            high_vote: Some(self.high_vote.build()),
            high_qc: Some(self.high_qc.build()),
            proposals: self.proposals.iter().map(|p| p.build()).collect(),
        }
    }
}

/// Iterator over missing block numbers.
pub(crate) struct MissingBlockNumbers<I: Iterator> {
    range: ops::Range<BlockNumber>,
    existing_numbers: iter::Peekable<I>,
}

impl<I> MissingBlockNumbers<I>
where
    I: Iterator<Item = anyhow::Result<BlockNumber>>,
{
    /// Creates a new iterator based on the provided params.
    pub(crate) fn new(range: ops::Range<BlockNumber>, existing_numbers: I) -> Self {
        Self {
            range,
            existing_numbers: existing_numbers.peekable(),
        }
    }
}

impl<I> Iterator for MissingBlockNumbers<I>
where
    I: Iterator<Item = anyhow::Result<BlockNumber>>,
{
    type Item = anyhow::Result<BlockNumber>;

    fn next(&mut self) -> Option<Self::Item> {
        // Loop while existing numbers match the starting numbers from the range. The check
        // that the range is non-empty is redundant given how `existing_numbers` are constructed
        // (they are guaranteed to be lesser than the upper range bound); we add it just to be safe.
        while !self.range.is_empty()
            && matches!(self.existing_numbers.peek(), Some(&Ok(num)) if num == self.range.start)
        {
            self.range.start = self.range.start.next();
            self.existing_numbers.next(); // Advance to the next number
        }

        if matches!(self.existing_numbers.peek(), Some(&Err(_))) {
            let err = self.existing_numbers.next().unwrap().unwrap_err();
            // ^ Both unwraps are safe due to the check above.
            return Some(Err(err));
        }

        if self.range.is_empty() {
            return None;
        }
        let next_number = self.range.start;
        self.range.start = self.range.start.next();
        Some(Ok(next_number))
    }
}

/// Storage errors.
#[derive(Debug, Error)]
pub enum StorageError {
    /// Operation was canceled by structured concurrency framework.
    #[error("operation was canceled by structured concurrency framework")]
    Canceled(#[from] ctx::Canceled),
    /// Database operation failed.
    #[error("database operation failed")]
    Database(#[source] anyhow::Error),
}

/// [`Result`] for fallible storage operations.
pub type StorageResult<T> = Result<T, StorageError>;
