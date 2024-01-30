use jsonrpsee::types::Params;

/// Trait to implement for new RPC methods.
pub(crate) trait RPCMethod {
    /// Method response logic when called.
    fn callback(params: Params) -> serde_json::Value;
    /// Method name.
    fn method() -> &'static str;
    /// Method path for GET requests.
    fn path() -> &'static str;
}

pub(crate) mod health_check;
