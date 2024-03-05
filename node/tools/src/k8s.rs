use crate::{
    config,
    network_chaos::{
        NetworkChaos, NetworkChaosAction, NetworkChaosDelay, NetworkChaosMode,
        NetworkChaosSelector, NetworkChaosSpec,
    },
    NodeAddr,
};
use anyhow::{anyhow, Context};
use k8s_openapi::{
    api::{
        apps::v1::{Deployment, DeploymentSpec},
        core::v1::{
            Container, Namespace, Pod, PodSpec, PodTemplateSpec, Service, ServicePort, ServiceSpec,
        },
        rbac::v1::{PolicyRule, Role, RoleBinding, RoleRef, Subject},
    },
    apimachinery::pkg::{apis::meta::v1::LabelSelector, util::intstr::IntOrString},
};
use kube::{
    api::{ListParams, PostParams},
    core::{ObjectList, ObjectMeta},
    Api, Client, ResourceExt,
};
use serde_json::json;
use std::{collections::HashMap, net::SocketAddr};
use tokio_retry::strategy::FixedInterval;
use tokio_retry::Retry;
use tracing::log::info;
use zksync_protobuf::serde::Serde;

/// Docker image name for consensus nodes.
const DOCKER_IMAGE_NAME: &str = "consensus-node";

/// K8s namespace for consensus nodes.
pub const DEFAULT_NAMESPACE: &str = "consensus";

/// Get a kube client
pub async fn get_client() -> anyhow::Result<Client> {
    Ok(Client::try_default().await?)
}

pub fn get_node_rpc_address(pod: Pod) -> anyhow::Result<SocketAddr> {
    let pod_spec = pod.spec.context("Failed to get pod spec")?;
    let pod_running_container = pod_spec
        .containers
        .first()
        .context("Failed to get pod container")?
        .to_owned();
    let port: u16 = pod_running_container
        .ports
        .context("Failed getting node rpc port")?
        .iter()
        .find_map(|port| {
            let port = port.container_port.try_into().ok()?;
            (port != config::NODES_PORT).then_some(port)
        })
        .context("Failed parsing node rpc port")?;
    let pod_ip = pod
        .status
        .clone()
        .context("Failed to get pod status")
        .ok()
        .and_then(|status| status.pod_ip.clone())
        .context("Failed to get pod ip")?;

    let pod_rpc_addr = SocketAddr::new(pod_ip.parse().context("Failed to parse pod ip")?, port);
    Ok(pod_rpc_addr)
}

pub async fn get_node_rpc_address_with_name(
    client: &Client,
    pod_id: &str,
) -> anyhow::Result<SocketAddr> {
    let pods: Api<Pod> = Api::namespaced(client.to_owned(), DEFAULT_NAMESPACE);
    let lp = ListParams::default().labels(&format!("id={}", pod_id));
    let pod = pods
        .list(&lp)
        .await?
        .items
        .first()
        .cloned()
        .context("Pod not found")?;
    let pod_rpc_addr = get_node_rpc_address(pod)?;
    Ok(pod_rpc_addr)
}

pub async fn create_or_reuse_network_chaos_role(client: &Client) -> anyhow::Result<()> {
    let roles: Api<Role> = Api::namespaced(client.to_owned(), "chaos-mesh");
    match roles.get_opt("network-chaos-reader").await? {
        Some(role) => {
            info!(
                "Role: {} ,already exists",
                role.metadata.name.context("Name not defined in metadata")?
            );
        }
        None => {
            let role = Role {
                metadata: ObjectMeta {
                    name: "network-chaos-reader".to_string().into(),
                    namespace: "chaos-mesh".to_owned().into(),
                    ..Default::default()
                },
                rules: vec![PolicyRule {
                    api_groups: Some(vec!["*".to_string()]),
                    resources: Some(vec!["networkchaos".to_string()]),
                    verbs: vec![
                        "get".to_owned(),
                        "list".to_owned(),
                        "watch".to_owned(),
                        "create".to_owned(),
                    ],
                    ..Default::default()
                }]
                .into(),
            };

            let roles: Api<Role> = Api::namespaced(client.to_owned(), "chaos-mesh");
            let post_params = PostParams::default();
            let result = roles.create(&post_params, &role).await?;

            info!(
                "Role: {} , created",
                result
                    .metadata
                    .name
                    .context("Name not defined in metadata")?
            );

            let role_binding = RoleBinding {
                metadata: ObjectMeta {
                    name: "network-chaos-reader-rule".to_string().into(),
                    namespace: "chaos-mesh".to_owned().into(),
                    ..Default::default()
                },
                subjects: vec![Subject {
                    kind: "ServiceAccount".to_string(),
                    name: "default".to_string(),
                    namespace: DEFAULT_NAMESPACE.to_owned().into(),
                    ..Default::default()
                }]
                .into(),
                role_ref: RoleRef {
                    api_group: "rbac.authorization.k8s.io".to_string(),
                    kind: "Role".to_string(),
                    name: "network-chaos-reader".to_string(),
                },
            };

            let role_bindings: Api<RoleBinding> = Api::namespaced(client.to_owned(), "chaos-mesh");
            let post_params = PostParams::default();
            let result = role_bindings.create(&post_params, &role_binding).await?;

            info!(
                "RoleBinding: {} , created",
                result
                    .metadata
                    .name
                    .context("Name not defined in metadata")?
            );
        }
    }

    Ok(())
}

pub async fn create_or_reuse_pod_reader_role(client: &Client) -> anyhow::Result<()> {
    let roles: Api<Role> = Api::namespaced(client.to_owned(), DEFAULT_NAMESPACE);
    match roles.get_opt("pod-reader").await? {
        Some(role) => {
            info!(
                "Role: {} ,already exists",
                role.metadata.name.context("Name not defined in metadata")?
            );
        }
        None => {
            let role = Role {
                metadata: ObjectMeta {
                    name: "pod-reader".to_string().into(),
                    namespace: DEFAULT_NAMESPACE.to_string().into(),
                    ..Default::default()
                },
                rules: vec![PolicyRule {
                    api_groups: Some(vec!["".to_string()]),
                    resources: Some(vec!["pods".to_string()]),
                    verbs: vec![
                        "get".to_owned(),
                        "list".to_owned(),
                        "watch".to_owned(),
                        "create".to_owned(),
                    ],
                    ..Default::default()
                }]
                .into(),
            };

            let roles: Api<Role> = Api::namespaced(client.to_owned(), DEFAULT_NAMESPACE);
            let post_params = PostParams::default();
            let result = roles.create(&post_params, &role).await?;

            info!(
                "Role: {} , created",
                result
                    .metadata
                    .name
                    .context("Name not defined in metadata")?
            );

            let role_binding = RoleBinding {
                metadata: ObjectMeta {
                    name: "pod-reader-rule".to_string().into(),
                    namespace: DEFAULT_NAMESPACE.to_string().into(),
                    ..Default::default()
                },
                role_ref: RoleRef {
                    api_group: "rbac.authorization.k8s.io".to_string(),
                    kind: "Role".to_string(),
                    name: "pod-reader".to_string(),
                },
                subjects: vec![Subject {
                    kind: "ServiceAccount".to_string(),
                    name: "default".to_string(),
                    api_group: "".to_owned().into(),
                    ..Default::default()
                }]
                .into(),
            };

            let role_bindings: Api<RoleBinding> =
                Api::namespaced(client.to_owned(), DEFAULT_NAMESPACE);
            let post_params = PostParams::default();
            let result = role_bindings.create(&post_params, &role_binding).await?;

            info!(
                "RoleBinding: {} , created",
                result
                    .metadata
                    .name
                    .context("Name not defined in metadata")?
            );
        }
    }

    Ok(())
}

/// Get the IP addresses and the exposed port of the RPC server of the consensus nodes in the kubernetes cluster.
pub async fn get_consensus_nodes_rpc_address(client: &Client) -> anyhow::Result<Vec<SocketAddr>> {
    let pods: Api<Pod> = Api::namespaced(client.to_owned(), DEFAULT_NAMESPACE);
    let lp = ListParams::default();
    let pods = pods.list(&lp).await?;
    let pod_addresses: anyhow::Result<Vec<SocketAddr>> = pods
        .into_iter()
        .map(|pod| get_node_rpc_address(pod))
        .collect();
    pod_addresses
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

pub async fn add_chaos_delay_for_node(client: &Client, node_name: &str) -> anyhow::Result<()> {
    let chaos = NetworkChaos::new(
        "chaos-delay",
        NetworkChaosSpec {
            action: NetworkChaosAction::Delay,
            mode: NetworkChaosMode::One,
            selector: NetworkChaosSelector {
                namespaces: vec![DEFAULT_NAMESPACE.to_string()].into(),
                label_selectors: Some([("app".to_string(), node_name.to_string())].into()),
            },
            delay: NetworkChaosDelay {
                latency: "1000ms".to_string(),
                ..Default::default()
            },
            duration: Some("10s".to_string()),
        },
    );

    let chaos_deployments = Api::namespaced(client.clone(), "chaos-mesh");
    let result = chaos_deployments
        .create(&PostParams::default(), &chaos)
        .await?;
    info!(
        "Chaos: {} , created",
        result
            .metadata
            .name
            .context("Name not defined in metadata")?
    );
    Ok(())
}

pub async fn create_tests_deployment(client: &Client) -> anyhow::Result<()> {
    let deployment: Deployment = Deployment {
        metadata: ObjectMeta {
            name: Some("tests-deployment".to_string()),
            namespace: Some(DEFAULT_NAMESPACE.to_string()),
            labels: Some([("app".to_string(), "test-node".to_string())].into()),
            ..Default::default()
        },
        spec: Some(DeploymentSpec {
            selector: LabelSelector {
                match_labels: Some([("app".to_string(), "test-node".to_string())].into()),
                ..Default::default()
            },
            replicas: Some(1),
            template: PodTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: Some([("app".to_string(), "test-node".to_string())].into()),
                    ..Default::default()
                }),
                spec: Some(PodSpec {
                    containers: vec![Container {
                        name: "test-suite".to_string(),
                        image: Some("test-suite:latest".to_string()),
                        image_pull_policy: Some("Never".to_string()),
                        command: Some(vec!["./tester_entrypoint.sh".to_string()]),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
            },
            ..Default::default()
        }),
        ..Default::default()
    };

    let deployments: Api<Deployment> = Api::namespaced(client.to_owned(), DEFAULT_NAMESPACE);
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

/// Creates a deployment
pub async fn deploy_node(
    client: &Client,
    node_index: usize,
    is_seed: bool,
    peers: Vec<NodeAddr>,
    namespace: &str,
) -> anyhow::Result<()> {
    let cli_args = get_cli_args(peers);
    let node_name = format!("consensus-node-{node_index:0>2}");
    let deployment: Deployment = serde_json::from_value(json!({
          "apiVersion": "apps/v1",
          "kind": "Deployment",
          "metadata": {
            "name": node_name,
            "namespace": namespace
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
                  "id": node_name,
                  "seed": is_seed.to_string()
                }
              },
              "spec": {
                "containers": [
                  {
                    "name": node_name,
                    "image": DOCKER_IMAGE_NAME,
                    "env": [
                      {
                        "name": "NODE_ID",
                        "value": node_name
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

/// Returns a HashMap with mapping: node_id -> IP address
pub async fn get_seed_node_addrs(
    client: &Client,
    amount: usize,
    namespace: &str,
) -> anyhow::Result<HashMap<String, String>> {
    let mut seed_nodes = HashMap::new();
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);

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
