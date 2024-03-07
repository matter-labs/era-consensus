//! Peers method for RPC server.
use jsonrpsee::core::RpcResult;
use std::sync::Arc;
use zksync_consensus_storage::BlockStore;

/// Config response for /config endpoint.
pub fn callback(node_storage: Arc<BlockStore>) -> RpcResult<serde_json::Value> {
    let sub = &mut node_storage.subscribe();
    let state = sub.borrow().clone();
    let a = state.last.unwrap().view().number;
    Ok(serde_json::json!({
        "last_view": a
    }))
}

/// Config method name.
pub fn method() -> &'static str {
    "last_view"
}

/// Method path for GET requests.
pub fn path() -> &'static str {
    "/last_view"
}
