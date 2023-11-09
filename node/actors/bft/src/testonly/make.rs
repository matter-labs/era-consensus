//! This module contains utilities that are only meant for testing purposes.

use crate::{
    io::{InputMessage, OutputMessage},
    Consensus,
};
use std::sync::Arc;
use zksync_concurrency::ctx;
use zksync_consensus_roles::validator;
use zksync_consensus_storage::{FallbackReplicaStateStore, InMemoryStorage};
use zksync_consensus_utils::pipe::{self, DispatcherPipe};

/// This creates a mock Consensus struct for unit tests.
pub async fn make_consensus(
    ctx: &ctx::Ctx,
    key: &validator::SecretKey,
    validator_set: &validator::ValidatorSet,
    genesis_block: &validator::FinalBlock,
) -> (Consensus, DispatcherPipe<InputMessage, OutputMessage>) {
    // Initialize the storage.
    let storage = InMemoryStorage::new(genesis_block.clone());
    // Create the pipe.
    let (consensus_pipe, dispatcher_pipe) = pipe::new();

    let consensus = Consensus::new(
        ctx,
        consensus_pipe,
        key.clone(),
        validator_set.clone(),
        FallbackReplicaStateStore::from_store(Arc::new(storage)),
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
    payload: validator::Payload,
) -> (validator::FinalBlock, validator::ValidatorSet) {
    let header = validator::BlockHeader::genesis(payload.hash());
    let validator_set = validator::ValidatorSet::new(keys.iter().map(|k| k.public())).unwrap();
    let signed_messages: Vec<_> = keys
        .iter()
        .map(|sk| {
            sk.sign_msg(validator::ReplicaCommit {
                protocol_version: validator::CURRENT_VERSION,
                view: validator::ViewNumber(0),
                proposal: header,
            })
        })
        .collect();
    let final_block = validator::FinalBlock {
        header,
        payload,
        justification: validator::CommitQC::from(&signed_messages, &validator_set).unwrap(),
    };
    (final_block, validator_set)
}
