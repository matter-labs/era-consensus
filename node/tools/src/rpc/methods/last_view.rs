//! Peers method for RPC server.
use anyhow::Context;
use jsonrpsee::{
    core::RpcResult,
    types::{error::ErrorCode, ErrorObjectOwned},
};
use std::sync::Arc;
use zksync_consensus_storage::BlockStore;

/// Last view response for /last_view endpoint.
pub fn callback(node_storage: Arc<BlockStore>) -> RpcResult<serde_json::Value> {
    let state = node_storage.queued();
    let last_view = state
        .last
        .context("Failed to get last state")
        .map_err(|_| ErrorObjectOwned::from(ErrorCode::InternalError))?
        .view()
        .number
        .0;
    // TODO(gprusak): this is the view of the last finalized block, not the current view of the
    // replica. Fix this.
    Ok(serde_json::json!({
        "last_view": last_view
    }))
}

/// Last view method name.
pub fn method() -> &'static str {
    "last_view"
}

/// Method path for GET requests.
pub fn path() -> &'static str {
    "/last_view"
}
