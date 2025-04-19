//! Peers method for RPC server.
use std::sync::Arc;

use anyhow::Context;
use jsonrpsee::{
    core::RpcResult,
    types::{error::ErrorCode, ErrorObjectOwned},
};
use zksync_consensus_engine::{EngineManager, Last};

/// Last view response for /last_view endpoint.
pub fn callback(node_engine: Arc<EngineManager>) -> RpcResult<serde_json::Value> {
    let state = node_engine.queued();
    let last_view = match state
        .last
        .context("Failed to get last state")
        .map_err(|_| ErrorObjectOwned::from(ErrorCode::InternalError))?
    {
        Last::PreGenesis(_) => 0,
        Last::FinalV1(qc) => qc.header().number.0,
        Last::FinalV2(qc) => qc.header().number.0,
    };
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
