//! Peers method for RPC server.
use crate::{config::encode_json, decode_json, AppConfig};

use super::RPCMethod;
use jsonrpsee::types::{error::ErrorCode, Params};
use std::fs::{self};
use zksync_protobuf::serde::Serde;

/// Config method for RPC server.
pub(crate) struct ConfigInfo;

// RPCMethod trait should be more general to allow external parameters like this case
// TODO fix the trait and implement this code in it
impl ConfigInfo {
    pub(crate) fn info(config: AppConfig) -> Result<serde_json::Value, ErrorCode> {
        // This may change in the future since we are assuming that the executor binary is being run inside the config directory.
        Ok(serde_json::json!({
            "config": encode_json(&Serde(config))
        }))
    }
}

impl RPCMethod for ConfigInfo {
    /// Config response for /config endpoint.
    fn callback(_params: Params) -> Result<serde_json::Value, ErrorCode> {
        // This may change in the future since we are assuming that the executor binary is being run inside the config directory.
        let node_config =
            fs::read_to_string("config.json").map_err(|_e| ErrorCode::InternalError)?;
        let node_config = decode_json::<Serde<AppConfig>>(&node_config)
            .map_err(|_e| ErrorCode::InternalError)?
            .0;
        let config = encode_json(&Serde(node_config));
        Ok(serde_json::json!({
            "config": config
        }))
    }

    /// Config method name.
    fn method() -> &'static str {
        "config"
    }

    /// Method path for GET requests.
    fn path() -> &'static str {
        "/config"
    }
}
