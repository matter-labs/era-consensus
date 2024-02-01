//! Peers method for RPC server.
use crate::{decode_json, AppConfig};

use super::RPCMethod;
use anyhow::Error;
use jsonrpsee::{
    types::{error::ErrorCode, ErrorObjectOwned, Params},
    MethodResponse,
};
use std::fs::{self};
use zksync_consensus_crypto::TextFmt;
use zksync_protobuf::serde::Serde;

/// Peers method for RPC server.
pub(crate) struct PeersInfo;

impl RPCMethod for PeersInfo {
    /// Peers response for /peers endpoint.
    fn callback(_params: Params) -> Result<serde_json::Value, ErrorCode> {
        // This may change in the future since we are assuming that the executor binary is being run inside the config directory.
        let node_config =
            fs::read_to_string("config.json").map_err(|_e| ErrorCode::InternalError)?;
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
    fn method() -> &'static str {
        "peers"
    }

    /// Method path for GET requests.
    fn path() -> &'static str {
        "/peers"
    }
}
