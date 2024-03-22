use kube::CustomResource;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
