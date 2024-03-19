use jsonrpsee::core::RpcResult;

/// Health check response for /health endpoint.
pub fn callback() -> RpcResult<serde_json::Value> {
    Ok(serde_json::json!({"health": true}))
}

/// Health check method name.
pub fn method() -> &'static str {
    "health_check"
}

/// Method path for GET requests.
pub fn path() -> &'static str {
    "/health"
}
