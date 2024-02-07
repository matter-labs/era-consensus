//! Tests for the block syncing actor.
use super::*;
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};
use std::ops;
use zksync_concurrency::{oneshot, time};
use zksync_consensus_network::io::GetBlockError;
use zksync_consensus_roles::validator::{self, testonly::GenesisSetup, BlockNumber, ValidatorSet};
use zksync_consensus_storage::{BlockStore, BlockStoreRunner, BlockStoreState};
use zksync_consensus_utils::pipe;

mod end_to_end;

const TEST_TIMEOUT: time::Duration = time::Duration::seconds(20);

impl Distribution<Config> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Config {
        let validator_set: ValidatorSet = rng.gen();
        let consensus_threshold = validator_set.len();
        Config::new(validator_set, consensus_threshold).unwrap()
    }
}

pub(crate) fn test_config(setup: &GenesisSetup) -> Config {
    Config::new(setup.genesis.validators.clone(), setup.keys.len()).unwrap()
}

pub(crate) fn sync_state(setup: &GenesisSetup, last_block_number: usize) -> BlockStoreState {
    snapshot_sync_state(setup, 1..=last_block_number)
}

pub(crate) fn snapshot_sync_state(
    setup: &GenesisSetup,
    range: ops::RangeInclusive<usize>,
) -> BlockStoreState {
    assert!(!range.is_empty());
    BlockStoreState {
        first: setup.blocks[*range.start()].justification.clone(),
        last: setup.blocks[*range.end()].justification.clone(),
    }
}

pub(crate) fn send_block(
    setup: &GenesisSetup,
    number: BlockNumber,
    response: oneshot::Sender<Result<validator::FinalBlock, GetBlockError>>,
) {
    let block = setup
        .blocks
        .get(number.0 as usize)
        .cloned()
        .ok_or(GetBlockError::NotAvailable);
    response.send(block).ok();
}
