//! Tests related to snapshot storage.

use super::*;
use crate::tests::{send_block, sync_state};
use zksync_consensus_network::io::GetBlockError;

#[tokio::test]
async fn backfilling_peer_history() {
    test_peer_states(BackfillingPeerHistory).await;
}
