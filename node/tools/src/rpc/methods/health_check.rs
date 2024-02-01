//! Health check method for RPC server.
use super::RPCMethod;
use jsonrpsee::types::{error::ErrorCode, ErrorObjectOwned, Params};

/// Health check method for RPC server.
pub(crate) struct HealthCheck;

impl RPCMethod for HealthCheck {
    /// Health check response for /health endpoint.
    fn callback(_params: Params) -> Result<serde_json::Value, ErrorCode> {
        Ok(serde_json::json!({"health": true}))
    }

    /// Health check method name.
    fn method() -> &'static str {
        "health_check"
    }

    /// Method path for GET requests.
    fn path() -> &'static str {
        "/health"
    }
}
