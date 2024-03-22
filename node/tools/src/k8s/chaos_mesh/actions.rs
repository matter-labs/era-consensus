use crate::k8s::chaos_mesh::cdr::{
    NetworkChaos, NetworkChaosAction, NetworkChaosDelay, NetworkChaosMode, NetworkChaosSelector,
    NetworkChaosSpec,
};
use crate::k8s::{self, PodId, DEFAULT_NAMESPACE};
use kube::api::{Api, DeleteParams, PostParams};

use anyhow::Context;
use kube::Client;
use tracing::log::info;

/// Create a network chaos resource to add delay to the network for a specific pod.
pub async fn add_chaos_delay_for_pod(client: &Client, pod_ip: PodId) -> anyhow::Result<()> {
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

pub async fn delete_chaos_delay_for_pod(pod_ip: &PodId) -> anyhow::Result<()> {
    let client = k8s::get_client().await?;
    let chaos_deployments: Api<NetworkChaos> = Api::namespaced(client.clone(), "chaos-mesh");
    let _ = chaos_deployments
        .delete(
            format!("chaos-delay-{}", pod_ip).as_str(),
            &DeleteParams::default(),
        )
        .await
        .map(|result| {
            result
                .map_left(|cdr| info!("Deleting {}", cdr.metadata.name.unwrap()))
                .map_right(|s| {
                    info!("Deleted: ({:?})", s);
                })
        });
    Ok(())
}
