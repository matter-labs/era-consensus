//! This is a simple test for the RPC server. It checks if the server is running and can respond to.
use std::{sync::Mutex, thread::sleep, time::Duration};

use clap::{Args, Parser, Subcommand};
use jsonrpsee::{
    core::{client::ClientT, RpcResult},
    http_client::HttpClientBuilder,
    rpc_params,
    server::{middleware::http::ProxyGetRequestLayer, Server},
    RpcModule,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::info;
use zksync_concurrency::{
    ctx::{self, Ctx},
    scope,
};
use zksync_consensus_tools::{
    k8s::{self, add_chaos_delay_for_node},
    rpc::methods::{health_check, last_vote},
};

mod utils;

/// Command line arguments.
#[derive(Debug, Parser)]
#[command(name = "tester")]
struct TesterCLI {
    /// Subcommand to run.
    #[command(subcommand)]
    command: TesterCommands,
}

/// Subcommands.
#[derive(Subcommand, Debug)]
enum TesterCommands {
    /// Set up the test pod.
    StartPod,
    /// Deploy the nodes.
    Run(RunArgs),
}

#[derive(Args, Debug)]
pub struct RunArgs {
    #[clap(long, default_value = "3030")]
    pub rpc_port: u16,
}

#[derive(Debug, Default)]
pub struct TestResult {
    passed: u16,
}

impl TestResult {
    fn add_passed(&mut self) {
        self.passed += 1;
    }
}

async fn run_test_rpc_server(
    ctx: &Ctx,
    port: u16,
    test_result: Arc<Mutex<TestResult>>,
) -> anyhow::Result<()> {
    let ip_address = SocketAddr::from(([0, 0, 0, 0], port));
    // Custom tower service to handle the RPC requests
    let service_builder = tower::ServiceBuilder::new()
        // Proxy `GET /<path>` requests to internal methods.
        .layer(ProxyGetRequestLayer::new("/test_result", "test_result")?);

    let mut module = RpcModule::new(());
    let test_result = test_result.clone();
    module.register_method("test_result", move |_params, _| {
        tests_status(test_result.clone())
    })?;

    let server = Server::builder()
        .set_http_middleware(service_builder)
        .build(ip_address)
        .await?;

    let handle = server.start(module);
    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(async {
            ctx.canceled().await;
            let _ = handle.stop();
            Ok(())
        });
        info!("Server started at {}", ip_address);
        handle.clone().stopped().await;
        Ok(())
    })
    .await
}

fn tests_status(counter: Arc<Mutex<TestResult>>) -> RpcResult<String> {
    Ok(format!("Tests passed: {}", counter.lock().unwrap().passed))
}

/// Sanity test for the RPC server.
/// We use unwraps here because this function is intended to be used like a test.
pub async fn sanity_test(test_result: Arc<Mutex<TestResult>>) -> anyhow::Result<()> {
    let client = k8s::get_client().await.unwrap();
    let nodes_socket = k8s::get_consensus_nodes_rpc_address(&client).await.unwrap();
    for socket in nodes_socket {
        let url: String = format!("http://{}", socket);
        let rpc_client = HttpClientBuilder::default().build(url).unwrap();
        let response: serde_json::Value = rpc_client
            .request(health_check::method(), rpc_params!())
            .await
            .unwrap();
        assert_eq!(response, health_check::callback().unwrap());
    }
    test_result.lock().unwrap().add_passed();
    Ok(())
}

/// Sanity test for the RPC server.
/// We use unwraps here because this function is intended to be used like a test.
pub async fn delay_test(test_result: Arc<Mutex<TestResult>>) -> anyhow::Result<()> {
    let client = k8s::get_client().await.unwrap();
    let target_node = "consensus-node-01";
    let ip = k8s::get_node_rpc_address_with_name(&client, target_node)
        .await
        .unwrap();
    add_chaos_delay_for_node(&client, target_node)
        .await
        .unwrap();
    let url: String = format!("http://{}", ip);
    let rpc_client = HttpClientBuilder::default().build(url).unwrap();
    let response: serde_json::Value = rpc_client
        .request(last_vote::method(), rpc_params!())
        .await
        .unwrap();
    let last_voted_view: u64 = serde_json::from_value(response).unwrap();
    info!("last_voted_view: {}", last_voted_view);
    for _ in 0..5 {
        let response: serde_json::Value = rpc_client
            .request(last_vote::method(), rpc_params!())
            .await
            .unwrap();
        let new_last_voted_view: u64 = serde_json::from_value(response).unwrap();
        info!("new_last_voted_view: {}", new_last_voted_view);
        assert_eq!(new_last_voted_view, last_voted_view);
    }
    sleep(Duration::from_secs(10));
    let response: serde_json::Value = rpc_client
        .request(last_vote::method(), rpc_params!())
        .await
        .unwrap();
    let new_last_voted_view: u64 = serde_json::from_value(response).unwrap();
    info!("new_last_voted_view: {}", new_last_voted_view);
    assert!(new_last_voted_view > last_voted_view);
    test_result.lock().unwrap().add_passed();
    Ok(())
}

/// Main function for the test.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let ctx = &ctx::root();
    tracing_subscriber::fmt::init();
    let args = TesterCLI::parse();
    let test_results = Arc::new(Mutex::new(TestResult::default()));

    match args.command {
        TesterCommands::StartPod => {
            utils::deploy_role().await?;
            utils::start_tests_pod().await?;
            utils::deploy_rpc_service().await
        }
        TesterCommands::Run(args) => {
            scope::run!(ctx, |ctx, s| async {
                s.spawn(run_test_rpc_server(
                    ctx,
                    args.rpc_port,
                    test_results.clone(),
                ));
                s.spawn(async {
                    sanity_test(test_results.clone()).await?;
                    delay_test(test_results.clone()).await
                });
                Ok(())
            })
            .await
        }
    }
}
