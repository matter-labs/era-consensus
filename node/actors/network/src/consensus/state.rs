use crate::pool::PoolWatch;
use std::collections::HashSet;
use zksync_consensus_roles::{validator, validator::ValidatorSet};

/// Configuration of the consensus network.
#[derive(Debug, Clone)]
pub struct Config {
    /// Private key of the validator. Currently only validator nodes
    /// are supported, but eventually it will become optional.
    pub key: validator::SecretKey,

    /// Public TCP address that other validators are expected to connect to.
    /// It is announced over gossip network.
    pub public_addr: std::net::SocketAddr,
}

/// Consensus network state.
pub(crate) struct State {
    /// Consensus configuration.
    pub(crate) cfg: Config,
    /// Set of the currently open inbound connections.
    pub(crate) inbound: PoolWatch<validator::PublicKey>,
    /// Set of the currently open outbound connections.
    pub(crate) outbound: PoolWatch<validator::PublicKey>,
}

impl State {
    /// Constructs a new State.
    pub(crate) fn new(cfg: Config, validators: &ValidatorSet) -> anyhow::Result<Self> {
        let validators: HashSet<_> = validators.iter().cloned().collect();
        let current_validator_key = cfg.key.public();
        anyhow::ensure!(
            validators.contains(&current_validator_key),
            "Validators' public keys {validators:?} do not contain the current validator \
             {current_validator_key:?}; this is not yet supported"
        );
        // ^ This check will be relaxed once we support dynamic validator membership

        Ok(Self {
            cfg,
            inbound: PoolWatch::new(validators.clone(), 0),
            outbound: PoolWatch::new(validators, 0),
        })
    }
}
