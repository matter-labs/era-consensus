use crate::configurator::Configs;
use anyhow::Context as _;
use crypto::Text;
use tracing::{debug, instrument};

/// This struct holds the file path to each of the config files.
#[derive(Debug)]
pub(crate) struct ConfigPaths {
    config: String,
    validator_key: String,
    node_key: String,
}

impl ConfigPaths {
    /// This function gets the paths for the config file and the key files.
    /// First, we try to get the path from the command line arguments. If that fails, we try to get
    /// it as an environment variable. If that also fails, we just use a default value.
    #[instrument(level = "trace")]
    pub(crate) fn resolve(args: &[String]) -> Self {
        Self {
            config: PathSpec {
                name: "Config file",
                flag: "--config-file",
                env: "CONFIG_FILE",
                default: "config.json",
            }
            .resolve(args),
            validator_key: PathSpec {
                name: "Validator key",
                flag: "--validator-key",
                env: "VALIDATOR_KEY",
                default: "validator_key",
            }
            .resolve(args),
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
        let cfg = Configs {
            config: protobuf_utils::decode_json(
                &std::fs::read_to_string(&self.config).context(self.config)?,
            )?,
            validator_key: Text::new(
                &std::fs::read_to_string(&self.validator_key).context(self.validator_key)?,
            )
            .decode()?,
            node_key: Text::new(&std::fs::read_to_string(&self.node_key).context(self.node_key)?)
                .decode()?,
        };
        if cfg.config.gossip.key != cfg.node_key.public() {
            anyhow::bail!(
                "config.gossip.key = {:?} doesn't match the secret key {:?}",
                cfg.config.gossip.key,
                cfg.node_key
            );
        }
        let public = cfg.config.consensus.key.clone();
        let secret = cfg.validator_key.clone();
        if public != secret.public() {
            anyhow::bail!(
                "config.consensus.key = {public:?} doesn't match the secret key {secret:?}"
            );
        }
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
            debug!("{} path found in command line arguments.", self.name);
            return path.clone();
        }

        if let Ok(path) = std::env::var(self.env) {
            debug!("{} path found in environment variable.", self.name);
            return path;
        }

        debug!("Using default {} path.", self.name);
        self.default.to_string()
    }
}

fn find_flag<'a>(args: &'a [String], flag: &'a str) -> Option<&'a String> {
    let pos = args.iter().position(|x| x == flag)?;
    args.get(pos + 1)
}
