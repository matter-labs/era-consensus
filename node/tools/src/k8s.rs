use crate::{config, NodeAddr};
use anyhow::{anyhow, Context};
use k8s_openapi::api::{
    apps::v1::Deployment,
    core::v1::{Namespace, Pod},
};
use kube::{
    api::{ListParams, PostParams},
    core::ObjectList,
    Api, Client, ResourceExt,
};
use serde_json::json;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use tokio_retry::strategy::FixedInterval;
use tokio_retry::Retry;
use tracing::log::info;
use zksync_consensus_roles::node;
use zksync_protobuf::serde::Serde;

/// Consensus Node Representation
#[derive(Debug)]
pub struct ConsensusNode {
    /// Node identifier
    pub id: String,
    /// Node key
    pub key: node::SecretKey,
    /// Full NodeAddr
    pub node_addr: Option<NodeAddr>,
    /// Is seed node (meaning it has no gossipStaticOutbound configuration)
    pub is_seed: bool,
    /// known gossipStaticOutbound peers
    pub gossip_static_outbound: Vec<NodeAddr>,
}

/// Get a kube client
pub async fn get_client() -> anyhow::Result<Client> {
    Ok(Client::try_default().await?)
}

/// Creates a namespace in k8s cluster
pub async fn create_or_reuse_namespace(client: &Client, name: &str) -> anyhow::Result<()> {
    let namespaces: Api<Namespace> = Api::all(client.clone());
    match namespaces.get_opt(name).await? {
        None => {
            let namespace: Namespace = serde_json::from_value(json!({
                "apiVersion": "v1",
                "kind": "Namespace",
                "metadata": {
                    "name": name,
                    "labels": {
                        "name": name
                    }
                }
            }))?;

            let namespaces: Api<Namespace> = Api::all(client.clone());
            let post_params = PostParams::default();
            let result = namespaces.create(&post_params, &namespace).await?;

            info!(
                "Namespace: {} ,created",
                result
                    .metadata
                    .name
                    .context("Name not defined in metadata")?
            );
            Ok(())
        }
        Some(consensus_namespace) => {
            info!(
                "Namespace: {} ,already exists",
                consensus_namespace
                    .metadata
                    .name
                    .context("Name not defined in metadata")?
            );
            Ok(())
        }
    }
}

/// Creates a deployment
pub async fn deploy_node(
    client: &Client,
    node: &ConsensusNode,
    namespace: &str,
) -> anyhow::Result<()> {
    let cli_args = get_cli_args(&node.gossip_static_outbound);
    let deployment: Deployment = serde_json::from_value(json!({
          "apiVersion": "apps/v1",
          "kind": "Deployment",
          "metadata": {
            "name": node.id,
            "namespace": namespace
          },
          "spec": {
            "selector": {
              "matchLabels": {
                "app": node.id
              }
            },
            "replicas": 1,
            "template": {
              "metadata": {
                "labels": {
                  "app": node.id,
                  "id": node.id,
                  "seed": node.is_seed.to_string()
                }
              },
              "spec": {
                "containers": [
                  {
                    "name": node.id,
                    "image": "consensus-node",
                    "env": [
                      {
                        "name": "NODE_ID",
                        "value": node.id
                      },
                      {
                        "name": "PUBLIC_ADDR",
                        "valueFrom": {
                            "fieldRef": {
                                "fieldPath": "status.podIP"
                            }
                        }
                      }
                    ],
                    "command": ["./k8s_entrypoint.sh"],
                    "args": cli_args,
                    "imagePullPolicy": "Never",
                    "ports": [
                      {
                        "containerPort": config::NODES_PORT
                      },
                      {
                        "containerPort": 3154
                      }
                    ],
                    "livenessProbe": {
                      "httpGet": {
                        "path": "/health",
                        "port": 3154
                      }
                    },
                    "readinessProbe": {
                      "httpGet": {
                        "path": "/health",
                        "port": 3154
                      }
                    }
                  }
                ]
              }
            }
          }
    }))?;

    let deployments: Api<Deployment> = Api::namespaced(client.clone(), namespace);
    let post_params = PostParams::default();
    let result = deployments.create(&post_params, &deployment).await?;

    info!(
        "Deployment: {} , created",
        result
            .metadata
            .name
            .context("Name not defined in metadata")?
    );
    Ok(())
}

/// Waits for the pods to start in order to complete each ConsensusNode with its assigned NodeAddr
pub async fn find_node_addrs(
    client: &Client,
    seed_nodes: &mut HashMap<String, &mut ConsensusNode>,
    namespace: &str,
) -> anyhow::Result<()> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);

    // Will retry 15 times during 15 seconds to allow pods to start and obtain an IP
    let retry_strategy = FixedInterval::from_millis(1000).take(15);
    let pod_list = Retry::spawn(retry_strategy, || get_seed_pods(&pods, seed_nodes.len())).await?;

    for p in pod_list {
        let id = p.labels()["id"].clone();
        let ip = p
            .status
            .context("Status not present")?
            .pod_ip
            .context("Pod IP address not present")?;
        let node: &mut ConsensusNode = &mut *seed_nodes.get_mut(&id).context("node not found")?;
        node.node_addr = Some(NodeAddr {
            key: node.key.public(),
            addr: SocketAddr::from_str(&format!("{}:{}", ip, config::NODES_PORT))?,
        });
    }
    Ok(())
}

async fn get_seed_pods(pods: &Api<Pod>, amount: usize) -> anyhow::Result<ObjectList<Pod>> {
    let lp = ListParams::default().labels("seed=true");
    let p = pods.list(&lp).await?;
    if p.items.len() == amount && p.iter().all(is_pod_running) {
        Ok(p)
    } else {
        Err(anyhow!("Pods are not ready"))
    }
}

fn is_pod_running(pod: &Pod) -> bool {
    if let Some(status) = &pod.status {
        if let Some(phase) = &status.phase {
            return phase == "Running";
        }
    }
    false
}

fn get_cli_args(peers: &Vec<NodeAddr>) -> Vec<String> {
    if peers.is_empty() {
        [].to_vec()
    } else {
        [
            "--add-gossip-static-outbound".to_string(),
            config::encode_with_serializer(
                &peers
                    .iter()
                    .map(|e| Serde(e.clone()))
                    .collect::<Vec<Serde<NodeAddr>>>(),
                serde_json::Serializer::new(vec![]),
            ),
        ]
        .to_vec()
    }
}
