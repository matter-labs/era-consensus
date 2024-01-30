//! Deployer for the kubernetes cluster.
use std::{fs, path::PathBuf};

use anyhow::Context;
use clap::{Parser, Subcommand};
use k8s_openapi::api::{apps::v1::Deployment, core::v1::Namespace};
use kube::{api::PostParams, Api, Client};
use rand::Rng;
use serde_json::json;
use tracing::log::info;
use zksync_consensus_crypto::TextFmt;
use zksync_consensus_roles::node;
use zksync_consensus_tools::AppConfig;

/// Command line arguments.
#[derive(Debug, Parser)]
#[command(name = "deployer")]
struct DeployerCLI {
    /// Subcommand to run.
    #[command(subcommand)]
    command: DeployerCommands,
}

/// Subcommand arguments.
#[derive(Debug, Parser)]
struct SubCommandArgs {
    /// Number of nodes to deploy.
    #[arg(long)]
    nodes: usize,
}

/// Subcommands.
#[derive(Subcommand, Debug)]
enum DeployerCommands {
    /// Generate configs for the nodes.
    GenerateConfig(SubCommandArgs),
    /// Deploy the nodes.
    Deploy(SubCommandArgs),
}

/// Generates config for the nodes to run in the kubernetes cluster
/// Creates a directory for each node in the parent k8s_configs directory.
fn generate_config(nodes: usize) -> anyhow::Result<()> {
    assert!(nodes > 0, "at least 1 node has to be specified");

    // Each node will have `gossip_peers` inbound peers.
    let peers = 2;

    // Generate the node keys for all the replicas.
    let rng = &mut rand::thread_rng();
    let node_keys: Vec<node::SecretKey> = (0..nodes).map(|_| rng.gen()).collect();

    let (default_config, validator_keys) = AppConfig::default_for(nodes);
    let mut cfgs: Vec<_> = (0..nodes).map(|_| default_config.clone()).collect();

    // Construct a gossip network with optimal diameter.
    for (i, node) in node_keys.iter().enumerate() {
        for j in 0..peers {
            let next = (i * peers + j + 1) % nodes;
            cfgs[next].add_gossip_static_inbound(node.public());
        }
    }

    let manifest_path = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let root = PathBuf::from(manifest_path).join("k8s_configs");
    let _ = fs::remove_dir_all(&root);
    for (i, cfg) in cfgs.into_iter().enumerate() {
        let node_config_dir = root.join(format!("node_{}", i));
        fs::create_dir_all(&node_config_dir)
            .with_context(|| format!("create_dir_all({:?})", node_config_dir))?;

        cfg.write_to_file(&node_config_dir)?;
        fs::write(
            node_config_dir.join("validator_key"),
            &TextFmt::encode(&validator_keys[i]),
        )
        .context("fs::write()")?;
        fs::write(
            node_config_dir.join("node_key"),
            &TextFmt::encode(&node_keys[i]),
        )
        .context("fs::write()")?;
    }

    Ok(())
}

/// Deploys the nodes to the kubernetes cluster.
async fn deploy(nodes: usize) -> anyhow::Result<()> {
    let client = Client::try_default().await?;

    let namespaces: Api<Namespace> = Api::all(client.clone());
    let consensus_namespace = namespaces.get_opt("consensus").await?;
    if consensus_namespace.is_none() {
        let namespace: Namespace = serde_json::from_value(json!({
            "apiVersion": "v1",
        "kind": "Namespace",
        "metadata": {
            "name": "consensus",
            "labels": {
                "name": "consensus"
            }
        }
        }))?;

        let namespaces: Api<Namespace> = Api::all(client.clone());
        let post_params = PostParams::default();
        let result = namespaces.create(&post_params, &namespace).await?;

        info!("Namespace: {} ,created", result.metadata.name.unwrap());
    } else {
        info!(
            "Namespace: {} ,already exists",
            consensus_namespace.unwrap().metadata.name.unwrap()
        );
    }

    for i in 0..nodes {
        let node_container_name = format!("consensus-node-0{}", i);
        let deployment: Deployment = serde_json::from_value(json!({
              "apiVersion": "apps/v1",
              "kind": "Deployment",
              "metadata": {
                "name": node_container_name,
                "namespace": "consensus"
              },
              "spec": {
                "selector": {
                  "matchLabels": {
                    "app": node_container_name
                  }
                },
                "replicas": 1,
                "template": {
                  "metadata": {
                    "labels": {
                      "app": node_container_name
                    }
                  },
                  "spec": {
                    "containers": [
                      {
                        "name": node_container_name,
                        "image": "consensus-node",
                        "env": [
                          {
                            "name": "NODE_ID",
                            "value": format!("node_{}", i)
                          }
                        ],
                        "command": ["./k8s_entrypoint.sh"],
                        "imagePullPolicy": "Never",
                        "ports": [
                          {
                            "containerPort": 3054
                          }
                        ],
                        "livenessProbe": {
                          "httpGet": {
                            "path": "/health",
                            "port": 3054
                          }
                        },
                        "readinessProbe": {
                          "httpGet": {
                            "path": "/health",
                            "port": 3054
                          }
                        }
                      }
                    ]
                  }
                }
              }
        }))?;

        let deployments: Api<Deployment> = Api::namespaced(client.clone(), "consensus");
        let post_params = PostParams::default();
        let result = deployments.create(&post_params, &deployment).await?;

        info!("Deployment: {} , created", result.metadata.name.unwrap());
    }

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let DeployerCLI { command } = DeployerCLI::parse();

    match command {
        DeployerCommands::GenerateConfig(args) => generate_config(args.nodes),
        DeployerCommands::Deploy(args) => deploy(args.nodes).await,
    }
}
