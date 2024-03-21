use crate::k8s::DEFAULT_NAMESPACE;
use kube::api::{Api, PostParams};
use kube::CustomResource;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
/// This role will allow the tester pod to deploy network chaos resources.
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

/// Create a network chaos resource to add delay to the network for a specific pod.
pub async fn add_chaos_delay_for_pod(
    client: &Client,
    pod_ip: PodId,
    duration_secs: u8,
) -> anyhow::Result<()> {
    let chaos = NetworkChaos::new(
        format!("chaos-delay-{}", pod_ip).as_str(),
        NetworkChaosSpec {
            action: NetworkChaosAction::Delay,
            mode: NetworkChaosMode::One,
            selector: NetworkChaosSelector {
                namespaces: vec![DEFAULT_NAMESPACE.to_string()].into(),
                label_selectors: Some([("app".to_string(), pod_ip.to_string())].into()),
            },
            delay: NetworkChaosDelay {
                latency: "1000ms".to_string(),
                ..Default::default()
            },
            duration: format!("{}s", duration_secs).into(),
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
