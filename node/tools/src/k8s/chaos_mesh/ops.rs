use crate::k8s::chaos_mesh::cdr::{
    NetworkChaos, NetworkChaosAction, NetworkChaosDelay, NetworkChaosMode, NetworkChaosSelector,
    NetworkChaosSpec,
};
use crate::k8s::{PodId, DEFAULT_NAMESPACE};
use kube::api::{Api, PostParams};

use anyhow::Context;
use k8s_openapi::api::rbac::v1::{PolicyRule, Role, RoleBinding, RoleRef, Subject};
use kube::{core::ObjectMeta, Client};
use tracing::log::info;

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
