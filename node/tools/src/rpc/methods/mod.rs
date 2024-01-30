use jsonrpsee::types::Params;

pub(crate) trait RPCMethod {
    fn callback(params: Params) -> serde_json::Value;
    fn method() -> &'static str;
    fn path() -> &'static str;
}

pub(crate) mod health_check;
