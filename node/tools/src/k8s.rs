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
use tokio_retry::strategy::FixedInterval;
use tokio_retry::Retry;
use tracing::log::info;
use zksync_protobuf::serde::Serde;

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
                    "name": "consensus",
                    "labels": {
                        "name": "consensus"
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
pub async fn create_deployment(
    client: &Client,
    node_number: usize,
    is_seed: bool,
    peers: Vec<NodeAddr>,
    namespace: &str,
) -> anyhow::Result<()> {
    let cli_args = get_cli_args(peers);
    let node_name = format!("consensus-node-{node_number:0>2}");
    let node_id = format!("node_{node_number:0>2}");
    let deployment: Deployment = serde_json::from_value(json!({
          "apiVersion": "apps/v1",
          "kind": "Deployment",
          "metadata": {
            "name": node_name,
            "namespace": "consensus"
          },
          "spec": {
            "selector": {
              "matchLabels": {
                "app": node_name
              }
            },
            "replicas": 1,
            "template": {
              "metadata": {
                "labels": {
                  "app": node_name,
                  "id": node_id,
                  "seed": is_seed.to_string()
                }
              },
              "spec": {
                "containers": [
                  {
                    "name": node_name,
                    "image": "consensus-node",
                    "env": [
                      {
                        "name": "NODE_ID",
                        "value": node_id
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
                        "containerPort": 3054
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

/// Returns a HashMap with mapping: node_id -> IP address
pub async fn get_seed_node_addrs(
    client: &Client,
    amount: usize,
) -> anyhow::Result<HashMap<String, String>> {
    let mut seed_nodes = HashMap::new();
    let pods: Api<Pod> = Api::namespaced(client.clone(), "consensus");

    // Will retry 15 times during 15 seconds to allow pods to start and obtain an IP
    let retry_strategy = FixedInterval::from_millis(1000).take(15);
    let pod_list = Retry::spawn(retry_strategy, || get_seed_pods(&pods, amount)).await?;

    for p in pod_list {
        let node_id = p.labels()["id"].clone();
        seed_nodes.insert(
            node_id,
            p.status
                .context("Status not present")?
                .pod_ip
                .context("Pod IP address not present")?,
        );
    }
    Ok(seed_nodes)
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

fn get_cli_args(peers: Vec<NodeAddr>) -> Vec<String> {
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
