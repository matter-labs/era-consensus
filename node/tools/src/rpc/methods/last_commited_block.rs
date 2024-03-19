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
    let sub = &mut node_storage.subscribe();
    let state = sub.borrow().clone();
    let last_commited_block_header = state
        .last
        .context("Failed to get last state")
        .map_err(|_| ErrorObjectOwned::from(ErrorCode::InternalError))?
        .header()
        .number
        .0;
    Ok(serde_json::json!({
        "last_commited_block": last_commited_block_header
    }))
}

/// Last view method name.
pub fn method() -> &'static str {
    "last_commited_block"
}

/// Method path for GET requests.
pub fn path() -> &'static str {
    "/last_commited_block"
}
