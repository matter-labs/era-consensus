//! Health check method for RPC server.
use jsonrpsee::types::Params;

/// Health check response for /health endpoint.
pub(crate) fn callback(_params: Params) -> serde_json::Value {
    serde_json::json!({"health": true})
}

/// Health check method name.
pub(crate) fn method() -> &'static str {
    "health_check"
}
