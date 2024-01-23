use jsonrpsee::types::Params;

pub fn callback(_params: Params) -> serde_json::Value {
    serde_json::json!({"health": true})
}

pub fn method() -> &'static str {
    "health_check"
}
