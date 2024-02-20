//! Peers method for RPC server.
use crate::{config::encode_json, decode_json, AppConfig};

use super::RPCMethod;
use jsonrpsee::types::{error::ErrorCode, Params};
use std::{fs, sync::Arc};
use zksync_consensus_storage::{BlockStore, ReplicaState};
use zksync_protobuf::serde::Serde;

/// Config method for RPC server.
pub(crate) struct LastView;

impl LastView {
    /// Provide the node's config information
    pub(crate) fn info(node_storage: Arc<BlockStore>) -> Result<serde_json::Value, ErrorCode> {
        let block = node_storage;
        let sub = &mut block.subscribe();
        let state = sub.borrow().clone();
        let replica_state = ReplicaState::from(state.last).view;
        let replica_state =
            serde_json::to_value(replica_state).map_err(|_e| ErrorCode::InternalError);
        replica_state
    }
}

impl RPCMethod for LastView {
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
        "last_view"
    }

    /// Method path for GET requests.
    fn path() -> &'static str {
        "/last_view"
    }
}
