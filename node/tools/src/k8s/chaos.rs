use crate::k8s::{get_last_view, DEFAULT_NAMESPACE};
use jsonrpsee::http_client::HttpClient;
use kube::api::{Api, PostParams};
use kube::CustomResource;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use zksync_concurrency::oneshot;
use zksync_consensus_roles::validator::{View, ViewNumber};

use crate::k8s;
use anyhow::Context;
use k8s_openapi::api::rbac::v1::{PolicyRule, Role, RoleBinding, RoleRef, Subject};
use kube::{core::ObjectMeta, Client};
use tracing::log::info;

use super::PodId;

/// Spec for the NetworkChaos CRD.
#[derive(CustomResource, Serialize, Deserialize, Clone, Debug, PartialEq)]
#[kube(
    group = "chaos-mesh.org",
    version = "v1alpha1",
    kind = "NetworkChaos",
    plural = "networkchaos"
)]
#[kube(namespaced)]
#[kube(schema = "disabled")]
pub struct NetworkChaosSpec {
    pub action: NetworkChaosAction,
    pub mode: NetworkChaosMode,
    pub selector: NetworkChaosSelector,
    pub delay: NetworkChaosDelay,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum NetworkChaosAction {
    #[serde(rename = "delay")]
    Delay,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct NetworkChaosSelector {
    pub namespaces: Option<Vec<String>>,
    pub label_selectors: Option<BTreeMap<String, String>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum NetworkChaosMode {
    #[serde(rename = "one")]
    One,
}

#[derive(Default, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct NetworkChaosDelay {
    pub latency: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jitter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation: Option<String>,
}

/// Create custom role and role binding for the tester pod.
/// This role will allow the tester pod to deploy and delete network chaos resources.
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
                        "delete".to_owned(),
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

pub struct ChaosScheduler {
    from_view: ViewNumber,
    to_view: ViewNumber,
    target_pods: Vec<PodId>,
}

impl ChaosScheduler {
    pub fn new(from_view: u64, to_view: u64, target_pods: Vec<PodId>) -> Self {
        Self {
            from_view: ViewNumber(from_view),
            to_view: ViewNumber(to_view),
            target_pods,
        }
    }

    async fn check_for_start_view(
        &self,
        client: &Client,
        rpc_client: HttpClient,
    ) -> anyhow::Result<()> {
        info!("Checking for start view");
        let (start_sender, start_receiver) = std::sync::mpsc::channel();
        let (start_view, end_view) = (self.from_view.0, self.to_view.0);
        let task = tokio::task::spawn(async move {
            let response = get_last_view(rpc_client).await.unwrap();
            info!("Last view: {}", response);
            if response >= start_view && response <= end_view {
                start_sender.send(()).unwrap();
            }
        });
        if start_receiver.recv().is_ok() {
            task.abort();
            for pod_id in self.target_pods.iter() {
                info!("Adding chaos delay for pod: {}", pod_id.0);
                add_chaos_delay_for_pod(&client, pod_id.clone()).await?;
            }
        }
        Ok(())
    }

    async fn check_for_end_view(
        &self,
        client: &Client,
        rpc_client: HttpClient,
    ) -> anyhow::Result<()> {
        info!("Checking for end view");
        let (end_sender, end_receiver) = std::sync::mpsc::channel();
        let end_view = self.to_view.0;
        let task = tokio::task::spawn(async move {
            let response = get_last_view(rpc_client).await.unwrap();
            info!("Last view: {}", response);
            if response >= end_view {
                end_sender.send(()).unwrap();
            }
        });
        if end_receiver.recv().is_ok() {
            task.abort();
            for pod_id in self.target_pods.iter() {
                info!("Deleting chaos delay for pod: {}", pod_id.0);
                remove_chaos_delay_for_pod(&client, pod_id.clone()).await?;
            }
        }
        Ok(())
    }

    pub async fn schedule_delay(&self) -> anyhow::Result<()> {
        let client = Client::try_default().await?;
        for pod_id in self.target_pods.iter() {
            let rpc_client = k8s::get_consensus_node_rpc_client(&pod_id).await?;
            self.check_for_start_view(&client, rpc_client.clone())
                .await?;
            info!("Waiting for end view");
            self.check_for_end_view(&client, rpc_client).await?;
        }
        Ok(())
    }
}

/// Create a network chaos resource to add delay to the network for a specific pod.
pub async fn add_chaos_delay_for_pod(client: &Client, node_name: PodId) -> anyhow::Result<()> {
    let chaos = NetworkChaos::new(
        &format!("chaos-delay-{}", node_name.0),
        NetworkChaosSpec {
            action: NetworkChaosAction::Delay,
            mode: NetworkChaosMode::One,
            selector: NetworkChaosSelector {
                namespaces: vec![DEFAULT_NAMESPACE.to_string()].into(),
                label_selectors: Some([("app".to_string(), node_name.0)].into()),
            },
            delay: NetworkChaosDelay {
                latency: "1000ms".to_string(),
                ..Default::default()
            },
            duration: None,
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

pub async fn remove_chaos_delay_for_pod(client: &Client, node_name: PodId) -> anyhow::Result<()> {
    let chaos_deployments: Api<NetworkChaos> = Api::namespaced(client.clone(), "chaos-mesh");
    let result = chaos_deployments
        .delete(&format!("chaos-delay-{}", node_name.0), &Default::default())
        .await?;
    if result.right().unwrap().is_success() {
        info!("Chaos for: {} , deleted", node_name.0);
    }
    Ok(())
}
