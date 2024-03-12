use crate::{config, AppConfig, NodeAddr};
use anyhow::{ensure, Context};
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
    core::ObjectMeta,
    Api, Client,
};
use std::{collections::BTreeMap, net::SocketAddr, time::Duration};
use tokio::time;
use tracing::log::info;
use zksync_consensus_crypto::TextFmt;
use zksync_consensus_roles::{node, validator};
use zksync_protobuf::serde::Serde;

/// Docker image name for consensus nodes.
const DOCKER_IMAGE_NAME: &str = "consensus-node";

/// K8s namespace for consensus nodes.
pub const DEFAULT_NAMESPACE: &str = "consensus";

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

impl ConsensusNode {
    /// Wait for a deployed consensus node to be ready and have an IP address
    pub async fn await_running_pod(
        &mut self,
        client: &Client,
        namespace: &str,
    ) -> anyhow::Result<Pod> {
        let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
        // Wait until the pod is running, otherwise we get an error.
        retry(15, Duration::from_millis(1000), || async {
            get_running_pod(&pods, &self.id).await
        })
        .await
    }

    /// Fetchs the pod's IP address and assignts to self.node_addr
    pub async fn fetch_and_assign_pod_ip(
        &mut self,
        client: &Client,
        namespace: &str,
    ) -> anyhow::Result<()> {
        let ip = self
            .await_running_pod(client, namespace)
            .await?
            .status
            .context("Status not present")?
            .pod_ip
            .context("Pod IP address not present")?;
        self.node_addr = Some(NodeAddr {
            key: self.key.public(),
            addr: SocketAddr::new(ip.parse()?, config::NODES_PORT),
        });
        Ok(())
    }

    /// Creates a deployment
    pub async fn deploy(&self, client: &Client, namespace: &str) -> anyhow::Result<()> {
        let cli_args = get_cli_args(self);
        let deployment = Deployment {
            metadata: ObjectMeta {
                name: Some(self.id.to_owned()),
                namespace: Some(namespace.to_owned()),
                ..Default::default()
            },
            spec: Some(DeploymentSpec {
                selector: LabelSelector {
                    match_labels: Some(BTreeMap::from([("app".to_owned(), self.id.to_owned())])),
                    ..Default::default()
                },
                replicas: Some(1),
                template: PodTemplateSpec {
                    metadata: Some(ObjectMeta {
                        labels: Some(BTreeMap::from([
                            ("app".to_owned(), self.id.to_owned()),
                            ("id".to_owned(), self.id.to_owned()),
                            ("seed".to_owned(), self.is_seed.to_string()),
                        ])),
                        ..Default::default()
                    }),
                    spec: Some(PodSpec {
                        containers: vec![Container {
                            name: self.id.to_owned(),
                            image: Some(DOCKER_IMAGE_NAME.to_owned()),
                            env: Some(vec![
                                EnvVar {
                                    name: "NODE_ID".to_owned(),
                                    value: Some(self.id.to_owned()),
                                    ..Default::default()
                                },
                                EnvVar {
                                    name: "PUBLIC_ADDR".to_owned(),
                                    value_from: Some(EnvVarSource {
                                        field_ref: Some(ObjectFieldSelector {
                                            field_path: "status.podIP".to_owned(),
                                            ..Default::default()
                                        }),
                                        ..Default::default()
                                    }),
                                    ..Default::default()
                                },
                            ]),
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
}

/// Get a kube client
pub async fn get_client() -> anyhow::Result<Client> {
    Ok(Client::try_default().await?)
}

/// Get the IP addresses and the exposed port of the RPC server of the consensus nodes in the kubernetes cluster.
pub async fn get_consensus_nodes_address(client: &Client) -> anyhow::Result<Vec<SocketAddr>> {
    let pods: Api<Pod> = Api::namespaced(client.to_owned(), DEFAULT_NAMESPACE);
    let lp = ListParams::default();
    let pods = pods.list(&lp).await?;
    ensure!(
        !pods.items.is_empty(),
        "No consensus pods found in the k8s cluster"
    );
    let mut node_rpc_addresses: Vec<SocketAddr> = Vec::new();
    for pod in pods.into_iter() {
        let pod_spec = pod.spec.as_ref().context("Failed to get pod spec")?;
        let pod_container = pod_spec
            .containers
            .first()
            .context("Failed to get container")?;
        if pod_container
            .image
            .as_ref()
            .context("Failed to get image")?
            .contains(DOCKER_IMAGE_NAME)
        {
            let pod_ip = pod
                .status
                .context("Failed to get pod status")?
                .pod_ip
                .context("Failed to get pod ip")?;
            let pod_rpc_port = pod_container
                .ports
                .as_ref()
                .context("Failed to get ports of container")?
                .iter()
                .find_map(|port| {
                    let port: u16 = port.container_port.try_into().ok()?;
                    (port != config::NODES_PORT).then_some(port)
                })
                .context("Failed parsing container port")?;
            node_rpc_addresses.push(SocketAddr::new(pod_ip.parse()?, pod_rpc_port));
        }
    }
    Ok(node_rpc_addresses)
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

async fn get_running_pod(pods: &Api<Pod>, label: &str) -> anyhow::Result<Pod> {
    let lp = ListParams::default().labels(&format!("app={label}"));
    let pod = pods
        .list(&lp)
        .await?
        .items
        .pop()
        .with_context(|| format!("Pod not found: {label}"))?;
    anyhow::ensure!(is_pod_running(&pod), "Pod is not running");
    Ok(pod)
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

async fn retry<T, Fut, F>(retries: usize, delay: Duration, mut f: F) -> anyhow::Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<T>>,
{
    let mut interval = time::interval(delay);
    let mut count = 0;
    loop {
        interval.tick().await;
        count += 1;
        let result = f().await;
        if result.is_ok() || count > retries {
            return result;
        }
    }
}
