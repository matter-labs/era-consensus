//! Health check method for RPC server.
use jsonrpsee::types::Params;

use super::RPCMethod;

pub(crate) struct HealthCheck;

impl RPCMethod for HealthCheck {
    /// Health check response for /health endpoint.
    fn callback(_params: Params) -> serde_json::Value {
        serde_json::json!({"health": true})
    }

    /// Health check method name.
    fn method() -> &'static str {
        "health_check"
    }

    fn path() -> &'static str {
        "/health"
    }
}
