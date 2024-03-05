//! Peers method for RPC server.
use jsonrpsee::core::RpcResult;
use std::sync::Arc;
use zksync_consensus_storage::{BlockStore, ReplicaState};

/// Config response for /config endpoint.
pub fn callback(node_storage: Arc<BlockStore>) -> RpcResult<serde_json::Value> {
    let sub = &mut node_storage.subscribe();
    let state = sub.borrow().clone();
    let replica_state = ReplicaState::from(state.last).high_vote.view;
    Ok(serde_json::json!(replica_state))
}

/// Config method name.
pub fn method() -> &'static str {
    "last_vote"
}

/// Method path for GET requests.
pub fn path() -> &'static str {
    "/last_vote"
}
