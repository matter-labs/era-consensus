//! Peers method for RPC server.
use crate::{decode_json, AppConfig};
use jsonrpsee::{core::RpcResult, types::error::ErrorCode};
use std::fs::{self};
use zksync_consensus_crypto::TextFmt;
use zksync_protobuf::serde::Serde;

/// Peers response for /peers endpoint.
pub fn callback() -> RpcResult<serde_json::Value> {
    // This may change in the future since we are assuming that the executor binary is being run inside the config directory.
    let node_config = fs::read_to_string("config.json").map_err(|_e| ErrorCode::InternalError)?;
    let node_config = decode_json::<Serde<AppConfig>>(&node_config)
        .map_err(|_e| ErrorCode::InternalError)?
        .0;
    let peers: Vec<String> = node_config
        .gossip_static_inbound
        .iter()
        .map(|x| x.encode())
        .collect();
    Ok(serde_json::json!({
        "peers": peers
    }))
}

/// Peers method name.
pub fn method() -> &'static str {
    "peers"
}

/// Method path for GET requests.
pub fn path() -> &'static str {
    "/peers"
}
