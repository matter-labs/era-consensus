//! Peers method for RPC server.
use crate::{config::encode_json, AppConfig};
use jsonrpsee::core::RpcResult;
use zksync_protobuf::serde::Serde;

/// Config response for /config endpoint.
pub fn callback(config: AppConfig) -> RpcResult<serde_json::Value> {
    // This may change in the future since we are assuming that the executor binary is being run inside the config directory.
    Ok(serde_json::json!({
        "config": encode_json(&Serde(config))
    }))
}

/// Config method name.
pub fn method() -> &'static str {
    "config"
}

/// Method path for GET requests.
pub fn path() -> &'static str {
    "/config"
}
