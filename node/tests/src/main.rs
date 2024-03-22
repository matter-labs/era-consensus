//! Binary containing the tests for the consensus nodes to run in the kubernetes cluster.
use clap::{Args, Parser, Subcommand};
use jsonrpsee::{
    core::RpcResult,
    server::{middleware::http::ProxyGetRequestLayer, Server},
    RpcModule,
};
use std::net::SocketAddr;
use std::sync::Arc;
use std::{sync::Mutex, thread::sleep, time::Duration};
use tracing::info;
use utils::get_consensus_nodes_rpc_client;
use zksync_concurrency::{
    ctx::{self, Ctx},
    scope,
};
use zksync_consensus_tools::k8s::{chaos_mesh::ops::delete_chaos_delay_for_pod, PodId};

use crate::utils::{
    add_chaos_delay_for_target_pods, check_health_of_node, get_consensus_node_rpc_client,
    get_last_commited_block,
};

mod utils;

/// CLI for the tester binary.
#[derive(Debug, Parser)]
#[command(name = "tester")]
struct TesterCLI {
    /// Subcommand to run.
    #[command(subcommand)]
    command: TesterCommands,
}

/// Subcommands for the `tester` binary.
#[derive(Subcommand, Debug)]
enum TesterCommands {
    /// Set up and deploy the test pod.
    StartPod,
    /// Run the tests and the RPC server with the test results.
    Run(RunArgs),
}

/// Arguments for the `run` subcommand.
#[derive(Args, Debug)]
pub struct RunArgs {
    /// Port for the RPC server to check the test results.
    #[clap(long, default_value = "3030")]
    pub rpc_port: u16,
}

/// Run the RPC server to check the test results.
async fn run_test_rpc_server(
    ctx: &Ctx,
    port: u16,
    test_result: Arc<Mutex<u8>>,
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

/// Method for the RPC server to check the test results.
fn tests_status(counter: Arc<Mutex<u8>>) -> RpcResult<String> {
    Ok(format!("Tests passed: {}", counter.lock().unwrap()))
}

/// Sanity test for the RPC server.
/// We use unwraps here because this function is intended to be used like a test.
pub async fn sanity_test(test_result: Arc<Mutex<u8>>) -> anyhow::Result<()> {
    let rpc_clients = get_consensus_nodes_rpc_client().await.unwrap();
    for rpc_client in rpc_clients {
        let response = check_health_of_node(rpc_client).await.unwrap();
        assert!(response);
    }
    *test_result.lock().unwrap() += 1;
    Ok(())
}

/// Delay test for the RPC server.
/// This tests introduce some delay in a specific node and checks if it recovers after some time.
/// We use unwraps here because this function is intended to be used like a test.
pub async fn delay_test(test_result: Arc<Mutex<u8>>) -> anyhow::Result<()> {
    let target_nodes = vec![PodId::from("consensus-node-01")];
    let rpc_client = get_consensus_node_rpc_client(target_nodes.first().unwrap())
        .await
        .unwrap();
    add_chaos_delay_for_target_pods(target_nodes.clone())
        .await
        .unwrap();
    let last_commited_block = get_last_commited_block(rpc_client.clone()).await.unwrap();
    delete_chaos_delay_for_pod(target_nodes.first().unwrap())
        .await
        .unwrap();
    // Wait for the deletion of the chaos delay.
    sleep(Duration::from_secs(2));
    let new_last_commited_block = get_last_commited_block(rpc_client).await.unwrap();
    assert!(new_last_commited_block > last_commited_block);
    *test_result.lock().unwrap() += 1;
    Ok(())
}

/// Main function for the test.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let ctx = &ctx::root();
    tracing_subscriber::fmt::init();
    let args = TesterCLI::parse();
    let test_passed = Arc::new(Mutex::new(0));

    match args.command {
        TesterCommands::StartPod => {
            utils::deploy_role().await?;
            utils::start_tests_pod().await?;
            utils::deploy_rpc_service().await
        }
        TesterCommands::Run(args) => {
            scope::run!(ctx, |ctx, s| async {
                s.spawn(run_test_rpc_server(ctx, args.rpc_port, test_passed.clone()));
                s.spawn(async {
                    sanity_test(test_passed.clone()).await?;
                    delay_test(test_passed.clone()).await
                });
                Ok(())
            })
            .await
        }
    }
}
