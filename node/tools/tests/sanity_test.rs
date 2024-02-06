mod tests {
    use jsonrpsee::{core::client::ClientT, http_client::HttpClientBuilder, rpc_params, types::Params};
    use kube::Api;
    use zksync_consensus_tools::rpc::methods::{health_check::HealthCheck, RPCMethod};
    use zksync_consensus_tools::k8s;
    use k8s_openapi::api::{
    };

    
    #[tokio::test]
    async fn sanity_test() {
        let url = "http://127.0.0.1:3154";
        //let k8s_client = k8s::get_client().await.unwrap();

        let rpc_client = HttpClientBuilder::default().build(url).unwrap();
        let params = Params::new(None);

        let response : serde_json::Value = rpc_client.request("health", rpc_params!()).await.unwrap();
        assert_eq!(response, HealthCheck::callback(params).unwrap());
    }
}
