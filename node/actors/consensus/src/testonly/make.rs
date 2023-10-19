//! This module contains utilities that are only meant for testing purposes.

use crate::{
    io::{InputMessage, OutputMessage},
    Consensus,
};
use concurrency::ctx;
use roles::validator;
use std::sync::Arc;
use storage::RocksdbStorage;
use tempfile::tempdir;
use utils::pipe::{self, DispatcherPipe};

/// This creates a mock Consensus struct for unit tests.
pub async fn make_consensus(
    ctx: &ctx::Ctx,
    key: &validator::SecretKey,
    validator_set: &validator::ValidatorSet,
    genesis_block: &validator::FinalBlock,
) -> (Consensus, DispatcherPipe<InputMessage, OutputMessage>) {
    // Create a temporary folder.
    let temp_dir = tempdir().unwrap();
    let temp_file = temp_dir.path().join("block_store");
    // Initialize the storage.
    let storage = RocksdbStorage::new(ctx, genesis_block, &temp_file)
        .await
        .unwrap();
    // Create the pipe.
    let (consensus_pipe, dispatcher_pipe) = pipe::new();

    let consensus = Consensus::new(
        ctx,
        consensus_pipe,
        key.clone(),
        validator_set.clone(),
        Arc::new(storage),
    );
    let consensus = consensus
        .await
        .expect("Initializing consensus actor failed");
    (consensus, dispatcher_pipe)
}

/// Creates a genesis block with the given payload
/// and a validator set for the chain.
pub fn make_genesis(
    keys: &[validator::SecretKey],
    payload: Vec<u8>,
) -> (validator::FinalBlock, validator::ValidatorSet) {
    let block = validator::Block::genesis(payload);
    let validator_set = validator::ValidatorSet::new(keys.iter().map(|k| k.public())).unwrap();
    let signed_messages: Vec<_> = keys
        .iter()
        .map(|sk| {
            sk.sign_msg(validator::ReplicaCommit {
                view: validator::ViewNumber(0),
                proposal_block_hash: block.hash(),
                proposal_block_number: validator::BlockNumber(0),
            })
        })
        .collect();
    let final_block = validator::FinalBlock {
        justification: validator::CommitQC::from(&signed_messages, &validator_set).unwrap(),
        block,
    };
    (final_block, validator_set)
}
