use crate::{config, NodeAddr};
use anyhow::Context;
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
use std::collections::BTreeMap;
use std::net::SocketAddr;
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

impl ConsensusNode {
    /// Wait for a deployed consensus node to be ready and have an IP address
    pub async fn await_running_pod(
        &mut self,
        client: &Client,
        namespace: &str,
    ) -> anyhow::Result<Pod> {
        let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
        // Wait until the pod is running, otherwise we get 500 error.
        let pod = Retry::spawn(FixedInterval::from_millis(1000).take(15), || {
            get_running_pod(&pods, &self.id)
        })
        .await?;
        Ok(pod)
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
        let cli_args = get_cli_args(&self.gossip_static_outbound);
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
                            image: Some("consensus-node".to_owned()),
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

async fn get_running_pod(pods: &Api<Pod>, label: &str) -> anyhow::Result<Pod> {
    let lp = ListParams::default().labels(&format!("app={label}"));
    let pod = pods
        .list(&lp)
        .await?
        .iter()
        .next()
        .with_context(|| format!("Pod not found: {label}"))
        .cloned()?;
    if is_pod_running(&pod) {
        Ok(pod)
    } else {
        Err(anyhow::format_err!("Pod not ready"))
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

fn get_cli_args(peers: &[NodeAddr]) -> Vec<String> {
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
