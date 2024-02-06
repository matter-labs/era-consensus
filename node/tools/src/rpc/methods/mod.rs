use jsonrpsee::types::{error::ErrorCode, Params};

/// Trait to implement for new RPC methods.
pub trait RPCMethod {
    /// Method response logic when called.
    fn callback(params: Params) -> Result<serde_json::Value, ErrorCode>;
    /// Method name.
    fn method() -> &'static str;
    /// Method path for GET requests.
    fn path() -> &'static str;
}

pub mod health_check;
pub(crate) mod peers;
