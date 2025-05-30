//! Peers method for RPC server.
use std::sync::Arc;

use anyhow::Context;
use jsonrpsee::{
    core::RpcResult,
    types::{error::ErrorCode, ErrorObjectOwned},
};
use zksync_consensus_engine::EngineManager;

/// Last view response for /last_view endpoint.
pub fn callback(node_engine: Arc<EngineManager>) -> RpcResult<serde_json::Value> {
    let state = node_engine.queued();
    let last_committed_block_header = state
        .last
        .context("Failed to get last state")
        .map_err(|_| ErrorObjectOwned::from(ErrorCode::InternalError))?
        .number()
        .0;
    Ok(serde_json::json!({
        "last_committed_block": last_committed_block_header
    }))
}

/// Last view method name.
pub fn method() -> &'static str {
    "last_committed_block"
}

/// Method path for GET requests.
pub fn path() -> &'static str {
    "/last_committed_block"
}
