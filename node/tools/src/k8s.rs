use futures::executor;
use k8s_openapi::api::{apps::v1::Deployment, core::v1::Namespace};
use kube::{api::PostParams, Api, Client};
use serde_json::json;
use tracing::log::info;

/// Get a kube client
pub fn get_client() -> anyhow::Result<Client> {
    Ok(executor::block_on(Client::try_default())?)
}

/// Creates a namespace in k8s cluster
pub fn create_or_reuse_namespace(client: &Client, name: &str) -> anyhow::Result<()> {
    let namespaces: Api<Namespace> = Api::all(client.clone());
    let consensus_namespace = executor::block_on(namespaces.get_opt(name))?;
    if consensus_namespace.is_none() {
        let namespace: Namespace = serde_json::from_value(json!({
            "apiVersion": "v1",
            "kind": "Namespace",
            "metadata": {
                "name": "consensus",
                "labels": {
                    "name": "consensus"
                }
            }
        }))?;

        let namespaces: Api<Namespace> = Api::all(client.clone());
        let post_params = PostParams::default();
        let result = executor::block_on(namespaces.create(&post_params, &namespace))?;

        info!("Namespace: {} ,created", result.metadata.name.unwrap());
        Ok(())
    } else {
        info!(
            "Namespace: {} ,already exists",
            consensus_namespace.unwrap().metadata.name.unwrap()
        );
        Ok(())
    }
}

pub fn create_deployment(
    client: &Client,
    node_name: &str,
    node_id: &str,
    namespace: &str,
) -> anyhow::Result<()> {
    let deployment: Deployment = serde_json::from_value(json!({
          "apiVersion": "apps/v1",
          "kind": "Deployment",
          "metadata": {
            "name": node_name,
            "namespace": "consensus"
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
                  "app": node_name
                }
              },
              "spec": {
                "containers": [
                  {
                    "name": node_name,
                    "image": "consensus-node",
                    "env": [
                      {
                        "name": "NODE_ID",
                        "value": node_id
                      }
                    ],
                    "command": ["./k8s_entrypoint.sh"],
                    "imagePullPolicy": "Never",
                    "ports": [
                      {
                        "containerPort": 3054
                      }
                    ],
                    "livenessProbe": {
                      "httpGet": {
                        "path": "/health",
                        "port": 3054
                      }
                    },
                    "readinessProbe": {
                      "httpGet": {
                        "path": "/health",
                        "port": 3054
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
    let result = executor::block_on(deployments.create(&post_params, &deployment))?;

    info!("Deployment: {} , created", result.metadata.name.unwrap());
    Ok(())
}
