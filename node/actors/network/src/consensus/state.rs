use crate::pool::PoolWatch;
use roles::validator;
use std::collections::HashSet;

/// Configuration of the consensus network.
#[derive(Clone)]
pub struct Config {
    /// Private key of the validator. Currently only validator nodes
    /// are supported, but eventually it will become optional.
    pub key: validator::SecretKey,

    /// Public TCP address that other validators are expected to connect to.
    /// It is announced over gossip network.
    pub public_addr: std::net::SocketAddr,

    /// Validators which
    /// * client should establish outbound connections to.
    /// * server should accept inbound connections from (1 per validator).
    pub validators: validator::ValidatorSet,
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
    pub(crate) fn new(cfg: Config) -> Self {
        let validators: HashSet<_> = cfg.validators.iter().cloned().collect();
        Self {
            cfg,
            inbound: PoolWatch::new(validators.clone(), 0),
            outbound: PoolWatch::new(validators, 0),
        }
    }
}
