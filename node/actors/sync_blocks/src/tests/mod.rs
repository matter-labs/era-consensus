//! Tests for the block syncing actor.
use super::*;
use zksync_concurrency::time;
use zksync_consensus_network::io::GetBlockError;
use zksync_consensus_roles::validator::{self, testonly::Setup};
use zksync_consensus_storage::{BlockStore, BlockStoreRunner, BlockStoreState};
use zksync_consensus_utils::pipe;

mod end_to_end;

const TEST_TIMEOUT: time::Duration = time::Duration::seconds(20);

pub(crate) fn sync_state(setup: &Setup, last: Option<&validator::FinalBlock>) -> BlockStoreState {
    BlockStoreState {
        first: setup.genesis.fork.first_block,
        last: last.map(|b| b.justification.clone()),
    }
}

pub(crate) fn make_response(
    block: Option<&validator::FinalBlock>,
) -> Result<validator::FinalBlock, GetBlockError> {
    block.cloned().ok_or(GetBlockError::NotAvailable)
}
