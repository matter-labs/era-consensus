use super::{Configs, NodeConfig};
use anyhow::Context as _;
use crypto::Text;
use roles::validator;
use std::fs;
use tracing::instrument;

/// This struct holds the file path to each of the config files.
#[derive(Debug)]
pub(crate) struct ConfigPaths {
    config: String,
    validator_key: Option<String>,
    node_key: String,
}

impl ConfigPaths {
    /// This function gets the paths for the config file and the key files.
    /// First, we try to get the path from the command line arguments. If that fails, we try to get
    /// it as an environment variable. If that also fails, we just use a default value.
    #[instrument(level = "trace")]
    pub(crate) fn resolve(args: &[String]) -> Self {
        let validator_key = PathSpec {
            name: "Validator key",
            flag: "--validator-key",
            env: "VALIDATOR_KEY",
            default: "validator_key",
        }
        .resolve(args);
        let validator_key = (!validator_key.is_empty()).then_some(validator_key);

        Self {
            config: PathSpec {
                name: "Config file",
                flag: "--config-file",
                env: "CONFIG_FILE",
                default: "config.json",
            }
            .resolve(args),
            validator_key,
            node_key: PathSpec {
                name: "Node key",
                flag: "--node-key",
                env: "NODE_KEY",
                default: "node_key",
            }
            .resolve(args),
        }
    }

    /// This function parses the config files from the paths given as command line arguments.
    #[instrument(level = "trace", ret)]
    pub(crate) fn read(self) -> anyhow::Result<Configs> {
        let node_config: NodeConfig =
            schema::decode_json(&fs::read_to_string(&self.config).context(self.config)?)?;
        let validator_key: Option<validator::SecretKey> = self
            .validator_key
            .as_ref()
            .map(|validator_key| {
                let read_key = fs::read_to_string(validator_key).context(validator_key.clone())?;
                Text::new(&read_key).decode()
            })
            .transpose()?;
        let node_key =
            Text::new(&fs::read_to_string(&self.node_key).context(self.node_key)?).decode()?;

        anyhow::ensure!(
            validator_key.is_some() == node_config.consensus.is_some(),
            "Validator key and consensus config must be specified at the same time"
        );
        let consensus = validator_key.and_then(|key| Some((node_config.consensus?, key)));

        let cfg = Configs {
            executor: node_config.executor,
            metrics_server_addr: node_config.metrics_server_addr,
            consensus,
            node_key,
        };
        Ok(cfg)
    }
}

#[derive(Debug)]
struct PathSpec<'a> {
    name: &'a str,
    flag: &'a str,
    env: &'a str,
    default: &'a str,
}

impl<'a> PathSpec<'a> {
    #[instrument(level = "trace", ret)]
    fn resolve(&self, args: &[String]) -> String {
        if let Some(path) = find_flag(args, self.flag) {
            tracing::debug!("{} path found in command line arguments.", self.name);
            return path.clone();
        }

        if let Ok(path) = std::env::var(self.env) {
            tracing::debug!("{} path found in environment variable.", self.name);
            return path;
        }

        tracing::debug!("Using default {} path.", self.name);
        self.default.to_string()
    }
}

fn find_flag<'a>(args: &'a [String], flag: &'a str) -> Option<&'a String> {
    let pos = args.iter().position(|x| x == flag)?;
    args.get(pos + 1)
}
