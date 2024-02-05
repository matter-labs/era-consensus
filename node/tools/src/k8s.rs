use crate::NodeAddr;
use k8s_openapi::api::{
    apps::v1::Deployment,
    core::v1::{Namespace, Pod},
};
use kube::{
    api::{ListParams, PostParams},
    Api, Client, ResourceExt,
};
use serde_json::json;
use std::collections::HashMap;
use tracing::log::info;
use zksync_protobuf::serde::Serde;

/// Get a kube client
pub async fn get_client() -> anyhow::Result<Client> {
    Ok(Client::try_default().await?)
}

/// Creates a namespace in k8s cluster
pub async fn create_or_reuse_namespace(client: &Client, name: &str) -> anyhow::Result<()> {
    let namespaces: Api<Namespace> = Api::all(client.clone());
    let consensus_namespace = namespaces.get_opt(name).await?;
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
        Ok(())
    } else {
        info!(
            "Namespace: {} ,already exists",
            consensus_namespace.unwrap().metadata.name.unwrap()
        );
        Ok(())
    }
}

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
                      }
                    ],
                    "command": ["./k8s_entrypoint.sh"],
                    "args": cli_args,
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

    let deployments: Api<Deployment> = Api::namespaced(client.clone(), namespace);
    let post_params = PostParams::default();
    let result = deployments.create(&post_params, &deployment).await?;

    info!("Deployment: {} , created", result.metadata.name.unwrap());
    Ok(())
}

/// Returns a HashMap with mapping: node_name -> IP address
pub async fn get_seed_node_addrs(client: &Client) -> HashMap<String, String> {
    let mut seed_nodes = HashMap::new();
    let pods: Api<Pod> = Api::namespaced(client.clone(), "consensus");

    let lp = ListParams::default().labels("seed=true");
    for p in pods.list(&lp).await.unwrap() {
        let node_id = p.labels()["id"].clone();
        seed_nodes.insert(node_id, p.status.unwrap().pod_ip.unwrap());
    }
    seed_nodes
}

fn get_cli_args(peers: Vec<NodeAddr>) -> Vec<String> {
    if peers.is_empty() {
        [].to_vec()
    } else {
        [
            "--add-gossip-static-outbound".to_string(),
            encode_json(
                &peers
                    .iter()
                    .map(|e| Serde(e.clone()))
                    .collect::<Vec<Serde<NodeAddr>>>(),
            ),
        ]
        .to_vec()
    }
}

/// Encodes a generated proto message to json for arbitrary ProtoFmt.
pub fn encode_json<T: serde::ser::Serialize>(x: &T) -> String {
    let mut s = serde_json::Serializer::new(vec![]);
    T::serialize(x, &mut s).unwrap();
    String::from_utf8(s.into_inner()).unwrap()
}
