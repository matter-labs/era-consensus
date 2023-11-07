//! Testonly utilities.
#![allow(dead_code)]
use crate::{consensus, event::Event, gossip, io::SyncState, Config, State};
use concurrency::{
    ctx,
    ctx::channel,
    io, net,
    sync::{self, watch},
};
use rand::Rng;
use roles::validator;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

/// Synchronously forwards data from one stream to another.
pub(crate) async fn forward(
    ctx: &ctx::Ctx,
    mut read: impl io::AsyncRead + Unpin,
    mut write: impl io::AsyncWrite + Unpin,
) {
    let mut buf = vec![0; 1024];
    let _: anyhow::Result<()> = async {
        loop {
            let n = io::read(ctx, &mut read, &mut buf).await??;
            if n == 0 {
                return Ok(());
            }
            io::write_all(ctx, &mut write, &buf[..n]).await??;
            io::flush(ctx, &mut write).await??;
        }
    }
    .await;
    // Normally a stream gets closed when it is dropped.
    // However in our tests we use io::split which effectively
    // keeps alive the stream until both halves are dropped.
    // Therefore we have to shut it down manually.
    let _ = io::shutdown(ctx, &mut write).await;
}

/// Node instance, wrapping the network actor state and the
/// events channel.
pub struct Instance {
    /// State of the instance.
    pub(crate) state: Arc<State>,
    /// Stream of events.
    pub(crate) events: channel::UnboundedReceiver<Event>,
}

impl Instance {
    /// Construct configs for `n` validators of the consensus.
    pub fn new_configs<R: Rng>(rng: &mut R, n: usize, gossip_peers: usize) -> Vec<Config> {
        let keys: Vec<validator::SecretKey> = (0..n).map(|_| rng.gen()).collect();
        let validators = validator::ValidatorSet::new(keys.iter().map(|k| k.public())).unwrap();
        let configs = keys.iter().map(|key| {
            let addr = net::tcp::testonly::reserve_listener();
            Config {
                server_addr: addr,
                validators: validators.clone(),
                consensus: Some(consensus::Config {
                    key: key.clone(),
                    public_addr: *addr,
                }),
                gossip: gossip::Config {
                    key: rng.gen(),
                    dynamic_inbound_limit: n as u64,
                    static_inbound: HashSet::default(),
                    static_outbound: HashMap::default(),
                    enable_pings: true,
                },
            }
        });
        let mut cfgs: Vec<_> = configs.collect();

        for i in 0..cfgs.len() {
            for j in 0..gossip_peers {
                let j = (i + j + 1) % n;
                let peer = cfgs[j].gossip.key.public();
                let addr = *cfgs[j].server_addr;
                cfgs[i].gossip.static_outbound.insert(peer, addr);
            }
        }
        cfgs
    }

    /// Construct an instance for a given config.
    pub(crate) fn from_cfg(cfg: Config) -> Self {
        let (events_send, events_recv) = channel::unbounded();
        Self {
            state: State::new(cfg, Some(events_send), None).expect("Invalid network config"),
            events: events_recv,
        }
    }

    /// Constructs `n` node instances, configured to connect to each other.
    /// Non-blocking (it doesn't run the network actors).
    pub fn new<R: Rng>(rng: &mut R, n: usize, gossip_peers: usize) -> Vec<Self> {
        Self::new_configs(rng, n, gossip_peers)
            .into_iter()
            .map(Self::from_cfg)
            .collect()
    }

    /// State getter.
    pub fn state(&self) -> &Arc<State> {
        &self.state
    }

    /// Returns the consensus config for this node, assuming it is a validator.
    pub fn consensus_config(&self) -> &consensus::Config {
        &self
            .state
            .consensus
            .as_ref()
            .expect("Node is not a validator")
            .cfg
    }

    /// Returns the gossip config for this node.
    pub fn gossip_config(&self) -> &gossip::Config {
        &self.state.gossip.cfg
    }

    /// Returns the overall config for this node.
    pub fn to_config(&self) -> Config {
        Config {
            server_addr: self.state.cfg.server_addr,
            validators: self.state.cfg.validators.clone(),
            gossip: self.state.gossip.cfg.clone(),
            consensus: self
                .state
                .consensus
                .as_ref()
                .map(|consensus_state| consensus_state.cfg.clone()),
        }
    }

    /// Sets a `SyncState` subscriber for the node. Panics if the node state is already shared.
    pub fn set_sync_state_subscriber(&mut self, sync_state: watch::Receiver<SyncState>) {
        let state = Arc::get_mut(&mut self.state).expect("node state is shared");
        state.gossip.sync_state = Some(sync_state);
    }

    /// Disables ping messages over the gossip network.
    pub fn disable_gossip_pings(&mut self) {
        let state = Arc::get_mut(&mut self.state).expect("node state is shared");
        state.gossip.cfg.enable_pings = false;
    }

    /// Wait for static outbound gossip connections to be established.
    pub async fn wait_for_gossip_connections(&self) {
        let gossip_state = &self.state.gossip;
        let want: HashSet<_> = gossip_state.cfg.static_outbound.keys().cloned().collect();
        gossip_state
            .outbound
            .subscribe()
            .wait_for(|got| want.is_subset(got.current()))
            .await
            .unwrap();
    }

    /// Waits for all the consensus connections to be established.
    pub async fn wait_for_consensus_connections(&self) {
        let consensus_state = self.state.consensus.as_ref().unwrap();

        let want: HashSet<_> = self.state.cfg.validators.iter().cloned().collect();
        consensus_state
            .inbound
            .subscribe()
            .wait_for(|got| got.current() == &want)
            .await
            .unwrap();
        consensus_state
            .outbound
            .subscribe()
            .wait_for(|got| got.current() == &want)
            .await
            .unwrap();
    }
}

/// Instantly broadcasts the ValidatorAddrs off-band
/// which triggers establishing consensus connections.
/// Useful for tests which don't care about bootstrapping the network
/// and just want to test business logic.
pub async fn instant_network(
    ctx: &ctx::Ctx,
    nodes: impl Iterator<Item = &Instance>,
) -> anyhow::Result<()> {
    // Collect validator addrs.
    let mut addrs = vec![];
    let nodes: Vec<_> = nodes.collect();
    for node in &nodes {
        let key = node.consensus_config().key.public();
        let sub = &mut node.state.gossip.validator_addrs.subscribe();
        loop {
            if let Some(addr) = sync::changed(ctx, sub).await?.get(&key) {
                addrs.push(addr.clone());
                break;
            }
        }
    }
    // Broadcast validator addrs.
    for node in &nodes {
        node.state
            .gossip
            .validator_addrs
            .update(&node.state.cfg.validators, &addrs)
            .await
            .unwrap();
    }
    // Wait for the connections.
    for n in &nodes {
        n.wait_for_consensus_connections().await;
    }
    tracing::info!("consensus network established");
    Ok(())
}

impl SyncState {
    /// Generates a random state based on the provided RNG and with the specified `last_stored_block`
    /// number.
    pub(crate) fn gen(rng: &mut impl Rng, number: validator::BlockNumber) -> Self {
        let mut this = Self {
            first_stored_block: rng.gen(),
            last_contiguous_stored_block: rng.gen(),
            last_stored_block: rng.gen(),
        };
        this.last_stored_block.message.proposal.number = number;
        this
    }
}
