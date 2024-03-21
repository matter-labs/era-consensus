use crate::config::AppConfig;
use crate::{config, NodeAddr};
use anyhow::Context;
use k8s_openapi::api::core::v1::Service;
use k8s_openapi::api::core::v1::ServicePort;
use k8s_openapi::api::core::v1::ServiceSpec;
use k8s_openapi::{
    api::{
        apps::v1::{Deployment, DeploymentSpec},
        core::v1::{
            Container, ContainerPort, EnvVar, EnvVarSource, HTTPGetAction, Namespace,
            ObjectFieldSelector, Pod, PodSpec, PodTemplateSpec, Probe,
        },
        rbac::v1::{PolicyRule, Role, RoleBinding, RoleRef, Subject},
    },
    apimachinery::pkg::{apis::meta::v1::LabelSelector, util::intstr::IntOrString::Int},
};
use kube::{
    api::{ListParams, PostParams},
    core::ObjectMeta,
    Api, Client,
};
use std::fmt::Display;
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

#[derive(Debug, Clone)]
pub struct PodId(pub String);

impl From<&str> for PodId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl Display for PodId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Consensus Node Representation
#[derive(Debug)]
pub struct ConsensusNode {
    /// Node identifier
    pub id: PodId,
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
            get_running_pod(&pods, &self.id.0).await
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
                name: Some(self.id.0.to_owned()),
                namespace: Some(namespace.to_owned()),
                ..Default::default()
            },
            spec: Some(DeploymentSpec {
                selector: LabelSelector {
                    match_labels: Some(BTreeMap::from([("app".to_owned(), self.id.0.to_owned())])),
                    ..Default::default()
                },
                replicas: Some(1),
                template: PodTemplateSpec {
                    metadata: Some(ObjectMeta {
                        labels: Some(BTreeMap::from([
                            ("app".to_owned(), self.id.0.to_owned()),
                            ("id".to_owned(), self.id.0.to_owned()),
                            ("seed".to_owned(), self.is_seed.to_string()),
                        ])),
                        ..Default::default()
                    }),
                    spec: Some(PodSpec {
                        containers: vec![Container {
                            name: self.id.0.to_owned(),
                            image: Some(DOCKER_IMAGE_NAME.to_owned()),
                            env: Some(vec![
                                EnvVar {
                                    name: "NODE_ID".to_owned(),
                                    value: Some(self.id.0.to_owned()),
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

pub async fn get_node_rpc_address_with_id(
    client: &Client,
    pod_id: &PodId,
) -> anyhow::Result<SocketAddr> {
    let pods: Api<Pod> = Api::namespaced(client.to_owned(), DEFAULT_NAMESPACE);
    let lp = ListParams::default().labels(&format!("id={}", pod_id.0));
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

/// Get the IP addresses and the exposed port of the RPC server of the consensus nodes in the kubernetes cluster.
pub async fn get_consensus_nodes_rpc_address(client: &Client) -> anyhow::Result<Vec<SocketAddr>> {
    let pods: Api<Pod> = Api::namespaced(client.to_owned(), DEFAULT_NAMESPACE);
    let lp = ListParams::default();
    let pods = pods.list(&lp).await?;
    let mut nodes_rpc_address = Vec::new();
    for pod in pods {
        if pod
            .spec
            .clone()
            .context("Failed to get pod spec")?
            .containers
            .first()
            .cloned()
            .context("Failed to get pod container")?
            .image
            .context("Failed to get pod image")?
            != DOCKER_IMAGE_NAME
        {
            continue;
        }
        let pod_rpc_addr = get_node_rpc_address(pod)?;
        nodes_rpc_address.push(pod_rpc_addr);
    }
    Ok(nodes_rpc_address)
}

fn is_pod_running(pod: &Pod) -> bool {
    if let Some(status) = &pod.status {
        if let Some(phase) = &status.phase {
            return phase == "Running";
        }
    }
    false
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

pub async fn expose_tester_rpc(client: &Client) -> anyhow::Result<()> {
    let load_balancer_sevice = Service {
        metadata: ObjectMeta {
            name: Some("tester-rpc".to_string()),
            namespace: Some(DEFAULT_NAMESPACE.to_string()),
            ..Default::default()
        },
        spec: Some(ServiceSpec {
            type_: Some("LoadBalancer".to_owned()),
            selector: Some([("app".to_string(), "test-node".to_string())].into()),
            ports: vec![ServicePort {
                port: 3030,
                target_port: Some(Int(3030)),
                ..Default::default()
            }]
            .into(),
            ..Default::default()
        }),
        ..Default::default()
    };

    let services: Api<Service> = Api::namespaced(client.clone(), DEFAULT_NAMESPACE);
    let post_params = PostParams::default();
    let result = services.create(&post_params, &load_balancer_sevice).await?;

    info!(
        "Service: {} , created",
        result
            .metadata
            .name
            .context("Name not defined in metadata")?
    );

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
                    api_groups: Some(vec![String::new()]),
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
                    api_group: String::new().into(),
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
    for count in 0.. {
        interval.tick().await;
        let result = f().await;
        if result.is_ok() || count > retries {
            return result;
        }
    }
    unreachable!("Loop should always return")
}
