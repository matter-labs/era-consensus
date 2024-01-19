use clap::{Args, Parser, Subcommand};
use node::node_client::NodeClient;
use node::HealthCheckRequest;

pub mod node {
    tonic::include_proto!("node");
}

pub const VERSION_STRING: &str = env!("CARGO_PKG_VERSION");

#[derive(Args)]
struct ClientConfig {
    #[arg(long)]
    server_address: String,
}

#[derive(Parser)]
#[command(name="client", author, version=VERSION_STRING, about, long_about = None)]
struct ClientCli {
    #[command(subcommand)]
    command: ClientCommands,
    #[clap(flatten)]
    config: ClientConfig,
}

#[derive(Subcommand)]
enum ClientCommands {
    #[command(name = "health_check")]
    HealthCheck,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ClientCli { command, config } = ClientCli::parse();
    let mut client = NodeClient::connect(config.server_address).await?;
    let response = match command {
        ClientCommands::HealthCheck => {
            let request = tonic::Request::new(HealthCheckRequest {});
            client.health_check(request).await?
        }
    };
    let res_message = response.into_inner().message;

    println!("RESPONSE={:?}", res_message);

    Ok(())
}
