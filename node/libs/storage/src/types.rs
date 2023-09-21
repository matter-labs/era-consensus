//! Defines the schema of the database.

use anyhow::Context as _;
use rocksdb::{Direction, IteratorMode};
use roles::validator;
use schema::{proto::storage as proto, read_required, ProtoFmt};
use std::collections::{BTreeMap, HashMap};

/// Enum used to represent a key in the database. It also acts as a separator between different stores.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DatabaseKey {
    /// Key used to store the replica state.
    /// ReplicaState -> ReplicaState
    ReplicaState,
    /// Key used to store the finalized blocks.
    /// Block(BlockNumber) -> FinalBlock
    Block(validator::BlockNumber),
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
    pub(crate) fn parse_block_key(raw_key: &[u8]) -> validator::BlockNumber {
        let raw_key = raw_key.try_into().expect("Invalid encoding for block key");
        validator::BlockNumber(u64::from_be_bytes(raw_key))
    }
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
    pub block_proposal_cache:
        BTreeMap<validator::BlockNumber, HashMap<validator::BlockHash, validator::Block>>,
}

impl ProtoFmt for ReplicaState {
    type Proto = proto::ReplicaState;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        let mut map: BTreeMap<_, HashMap<_, _>> = BTreeMap::new();

        for schema_block in r.blocks.iter() {
            let block: validator::Block =
                read_required(&Some(schema_block).cloned()).context("block")?;

            match map.get_mut(&block.number) {
                Some(blocks) => {
                    blocks.insert(block.hash(), block.clone());
                }
                None => {
                    let mut blocks = HashMap::new();
                    blocks.insert(block.hash(), block.clone());
                    map.insert(block.number, blocks);
                }
            }
        }

        Ok(Self {
            view: validator::ViewNumber(r.view.context("view_number")?),
            phase: read_required(&r.phase).context("phase")?,
            high_vote: read_required(&r.high_vote).context("high_vote")?,
            high_qc: read_required(&r.high_qc).context("high_qc")?,
            block_proposal_cache: map,
        })
    }

    fn build(&self) -> Self::Proto {
        let blocks = self
            .block_proposal_cache
            .values()
            .flat_map(|x| x.values())
            .map(|block| block.build())
            .collect();

        Self::Proto {
            view: Some(self.view.0),
            phase: Some(self.phase.build()),
            high_vote: Some(self.high_vote.build()),
            high_qc: Some(self.high_qc.build()),
            blocks,
        }
    }
}
