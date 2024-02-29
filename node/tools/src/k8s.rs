use crate::{config, AppConfig, NodeAddr};
use anyhow::{anyhow, Context};
use k8s_openapi::{
    api::{
        apps::v1::{Deployment, DeploymentSpec},
        core::v1::{
            Container, ContainerPort, EnvVar, EnvVarSource, HTTPGetAction, Namespace,
            ObjectFieldSelector, Pod, PodSpec, PodTemplateSpec, Probe,
        },
    },
    apimachinery::pkg::{apis::meta::v1::LabelSelector, util::intstr::IntOrString::Int},
};
use kube::{
    api::{ListParams, PostParams},
    core::{ObjectList, ObjectMeta},
    Api, Client, ResourceExt,
};
use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::str::FromStr;
use tokio_retry::strategy::FixedInterval;
use tokio_retry::Retry;
use tracing::log::info;
use zksync_consensus_crypto::TextFmt;
use zksync_consensus_roles::{node, validator};
use zksync_protobuf::serde::Serde;

/// Consensus Node Representation
#[derive(Debug)]
pub struct ConsensusNode {
    /// Node identifier
    pub id: String,
    /// Node configuration
    pub config: AppConfig,
    /// Node key
    pub key: node::SecretKey,
    /// Node key
    pub validator_key: Option<validator::SecretKey>,
    /// Full NodeAddr
    pub node_addr: Option<NodeAddr>,
    /// Is seed node (meaning it has no gossipStaticOutbound configuration)
    pub is_seed: bool,
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
            let namespace = Namespace {
                metadata: ObjectMeta {
                    name: Some(name.to_owned()),
                    labels: Some(BTreeMap::from([("name".to_owned(), name.to_owned())])),
                    ..Default::default()
                },
                ..Default::default()
            };

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
    let cli_args = get_cli_args(node);
    let deployment = Deployment {
        metadata: ObjectMeta {
            name: Some(node.id.to_owned()),
            namespace: Some(namespace.to_owned()),
            ..Default::default()
        },
        spec: Some(DeploymentSpec {
            selector: LabelSelector {
                match_labels: Some(BTreeMap::from([("app".to_owned(), node.id.to_owned())])),
                ..Default::default()
            },
            replicas: Some(1),
            template: PodTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: Some(BTreeMap::from([
                        ("app".to_owned(), node.id.to_owned()),
                        ("id".to_owned(), node.id.to_owned()),
                        ("seed".to_owned(), node.is_seed.to_string()),
                    ])),
                    ..Default::default()
                }),
                spec: Some(PodSpec {
                    containers: vec![Container {
                        name: node.id.to_owned(),
                        image: Some("consensus-node".to_owned()),
                        env: Some(vec![EnvVar {
                            name: "PUBLIC_ADDR".to_owned(),
                            value_from: Some(EnvVarSource {
                                field_ref: Some(ObjectFieldSelector {
                                    field_path: "status.podIP".to_owned(),
                                    ..Default::default()
                                }),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }]),
                        command: Some(vec!["./k8s_entrypoint.sh".to_owned()]),
                        args: Some(cli_args),
                        image_pull_policy: Some("Never".to_owned()),
                        ports: Some(vec![
                            ContainerPort {
                                container_port: i32::from(config::NODES_PORT),
                                ..Default::default()
                            },
                            ContainerPort {
                                container_port: 3154,
                                ..Default::default()
                            },
                        ]),
                        liveness_probe: Some(Probe {
                            http_get: Some(HTTPGetAction {
                                path: Some("/health".to_owned()),
                                port: Int(3154),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }),
                        readiness_probe: Some(Probe {
                            http_get: Some(HTTPGetAction {
                                path: Some("/health".to_owned()),
                                port: Int(3154),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
            },
            ..Default::default()
        }),
        ..Default::default()
    };

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
    nodes: &mut HashMap<String, &mut ConsensusNode>,
    namespace: &str,
) -> anyhow::Result<()> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);

    // Will retry 15 times during 15 seconds to allow pods to start and obtain an IP
    let retry_strategy = FixedInterval::from_millis(1000).take(15);
    let pod_list = Retry::spawn(retry_strategy, || get_seed_pods(&pods, nodes.len())).await?;

    for p in pod_list {
        let id = p.labels()["id"].clone();
        let ip = p
            .status
            .context("Status not present")?
            .pod_ip
            .context("Pod IP address not present")?;
        let node: &mut ConsensusNode = *nodes.get_mut(&id).context("node not found")?;
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

fn get_cli_args(consensus_node: &ConsensusNode) -> Vec<String> {
    let mut cli_args = [
        "--config".to_string(),
        config::encode_with_serializer(
            &Serde(consensus_node.config.clone()),
            serde_json::Serializer::new(vec![]),
        ),
        "--node-key".to_string(),
        TextFmt::encode(&consensus_node.key),
    ]
    .to_vec();
    if let Some(key) = &consensus_node.validator_key {
        cli_args.append(&mut ["--validator-key".to_string(), TextFmt::encode(key)].to_vec())
    };
    cli_args
}
