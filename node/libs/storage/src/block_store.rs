//! This module contains the methods to handle an append-only database of finalized blocks. Since we only store finalized blocks, this forms a
//! chain of blocks, not a tree (assuming we have all blocks and not have any gap). It allows for basic functionality like inserting a block,
//! getting a block, checking if a block is contained in the DB. We also store the head of the chain. Storing it explicitly allows us to fetch
//! the current head quickly.

use crate::{types::DatabaseKey, Storage};
use rocksdb::{IteratorMode, ReadOptions};
use roles::validator::{BlockNumber, FinalBlock};
use std::{iter, ops, sync::atomic::Ordering};

impl Storage {
    // ---------------- Read methods ----------------

    /// Gets the head block.
    pub fn get_head_block(&self) -> FinalBlock {
        let db = self.read();

        let mut options = ReadOptions::default();
        options.set_iterate_range(DatabaseKey::BLOCKS_START_KEY..);
        let mut iter = db.iterator_opt(DatabaseKey::BLOCK_HEAD_ITERATOR, options);
        let (_, head_block) = iter
            .next()
            .expect("Head block not found")
            .expect("RocksDB error reading head block");
        schema::decode(&head_block).expect("Failed decoding head block bytes")
    }

    /// Returns a block with the least number stored in this database.
    pub fn get_first_block(&self) -> FinalBlock {
        let db = self.read();

        let mut options = ReadOptions::default();
        options.set_iterate_range(DatabaseKey::BLOCKS_START_KEY..);
        let mut iter = db.iterator_opt(IteratorMode::Start, options);
        let (_, first_block) = iter
            .next()
            .expect("First stored block not found")
            .expect("RocksDB error reading first stored block");
        schema::decode(&first_block).expect("Failed decoding first stored block bytes")
    }

    /// Returns the number of the last block in the first contiguous range of blocks stored in this DB.
    /// If there are no missing blocks, this is equal to the number of [`Self::get_head_block()`],
    /// if there *are* missing blocks, the returned number will be lower.
    pub fn get_last_contiguous_block_number(&self) -> BlockNumber {
        let last_contiguous_block_number = self
            .cached_last_contiguous_block_number
            .load(Ordering::Relaxed);
        let last_contiguous_block_number = BlockNumber(last_contiguous_block_number);

        let last_contiguous_block_number =
            self.last_contiguous_block_number_impl(last_contiguous_block_number);

        // The cached value may have been updated by the other thread. Fortunately, we have a simple
        // protection against such "edit conflicts": the greater cached value is always valid and
        // should win.
        self.cached_last_contiguous_block_number
            .fetch_max(last_contiguous_block_number.0, Ordering::Relaxed);
        last_contiguous_block_number
    }

    // Implementation that is not aware of caching specifics. The only requirement for the method correctness
    // is for the `cached_last_contiguous_block_number` to be present in the database.
    fn last_contiguous_block_number_impl(
        &self,
        cached_last_contiguous_block_number: BlockNumber,
    ) -> BlockNumber {
        let db = self.read();

        let mut options = ReadOptions::default();
        let start_key = DatabaseKey::Block(cached_last_contiguous_block_number).encode_key();
        options.set_iterate_range(start_key..);
        let iter = db.iterator_opt(IteratorMode::Start, options);
        let iter = iter
            .map(|bytes| {
                let (key, _) = bytes.expect("RocksDB error iterating over block numbers");
                DatabaseKey::parse_block_key(&key)
            })
            .fuse();

        let mut prev_block_number = cached_last_contiguous_block_number;
        for block_number in iter {
            if block_number > prev_block_number.next() {
                return prev_block_number;
            }
            prev_block_number = block_number;
        }
        prev_block_number
    }

    /// Gets a block by its number.
    pub fn get_block(&self, number: BlockNumber) -> Option<FinalBlock> {
        let db = self.read();

        let raw_block = db
            .get(DatabaseKey::Block(number).encode_key())
            .unwrap_or_else(|err| panic!("RocksDB error reading block #{number}: {err}"))?;
        Some(schema::decode(&raw_block).unwrap_or_else(|err| {
            panic!("Failed decoding block #{number}: {err}");
        }))
    }

    /// Iterates over block numbers in the specified `range` that the DB *does not* have.
    // TODO(slowli): We might want to limit the length of the vec returned
    pub fn get_missing_block_numbers(&self, range: ops::Range<BlockNumber>) -> Vec<BlockNumber> {
        let db = self.read();

        let mut options = ReadOptions::default();
        let start_key = DatabaseKey::Block(range.start).encode_key();
        let end_key = DatabaseKey::Block(range.end).encode_key();
        options.set_iterate_range(start_key..end_key);

        let iter = db.iterator_opt(IteratorMode::Start, options);
        let iter = iter
            .map(|bytes| {
                let (key, _) = bytes.expect("RocksDB error iterating over block numbers");
                DatabaseKey::parse_block_key(&key)
            })
            .fuse();

        MissingBlockNumbers {
            range,
            existing_numbers: iter.peekable(),
        }
        .collect()
    }

    // ---------------- Write methods ----------------

    /// Insert a new block into the database.
    pub fn put_block(&self, finalized_block: &FinalBlock) {
        let db = self.write();

        let block_number = finalized_block.block.number;
        tracing::debug!("Inserting new block #{block_number} into the database.");

        let mut write_batch = rocksdb::WriteBatch::default();
        write_batch.put(
            DatabaseKey::Block(block_number).encode_key(),
            schema::encode(finalized_block),
        );

        // Commit the transaction.
        db.write(write_batch).unwrap();
        drop(db);

        self.block_writes_sender.send_replace(block_number);
    }
}

struct MissingBlockNumbers<I: Iterator> {
    range: ops::Range<BlockNumber>,
    existing_numbers: iter::Peekable<I>,
}

impl<I> Iterator for MissingBlockNumbers<I>
where
    I: Iterator<Item = BlockNumber>,
{
    type Item = BlockNumber;

    fn next(&mut self) -> Option<Self::Item> {
        // Loop while existing numbers match the starting numbers from the range. The check
        // that the range is non-empty is redundant given how `existing_numbers` are constructed
        // (they are guaranteed to be lesser than the upper range bound); we add it just to be safe.
        while !self.range.is_empty() && self.existing_numbers.peek() == Some(&self.range.start) {
            self.range.start = self.range.start.next();
            self.existing_numbers.next(); // Advance to the next number
        }

        if self.range.is_empty() {
            return None;
        }
        let next_number = self.range.start;
        self.range.start = self.range.start.next();
        Some(next_number)
    }
}
