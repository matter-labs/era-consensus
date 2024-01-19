use tonic::{transport::Server, Request, Response, Status};

use node::node_server::{Node, NodeServer};
use node::{HealthCheckRequest, HealthCheckResponse};

pub mod node {
    tonic::include_proto!("node");
}

#[derive(Debug, Default)]
pub struct MyNode {}

#[tonic::async_trait]
impl Node for MyNode {
    async fn health_check(
        &self,
        _request: Request<HealthCheckRequest>,
    ) -> Result<Response<HealthCheckResponse>, Status> {
        let reply = HealthCheckResponse {
            message: format!("Live").into(),
        };

        Ok(Response::new(reply))
    }
}

pub struct NodeRpcServer {
    ip_address: String,
}

impl NodeRpcServer {
    pub fn new(ip_address: String) -> Self {
        Self { ip_address }
    }

    pub async fn run(&self) -> anyhow::Result<()> {
        let addr = self.ip_address.parse()?;
        let node = MyNode::default();
    
        Server::builder()
            .add_service(NodeServer::new(node))
            .serve(addr)
            .await?;
    
        Ok(())
    }
}
