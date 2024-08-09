//! Testonly utilities.
#![allow(dead_code)]
use crate::{
    gossip::attestation,
    io::{ConsensusInputMessage, Target},
    Config, GossipConfig, Network, RpcConfig, Runner,
};
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use zksync_concurrency::{
    ctx::{self, channel},
    io, limiter, net, scope, sync,
};
use zksync_consensus_roles::{node, validator};
use zksync_consensus_storage::{BatchStore, BlockStore};
use zksync_consensus_utils::pipe;

impl Distribution<Target> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Target {
        match rng.gen_range(0..2) {
            0 => Target::Broadcast,
            _ => Target::Validator(rng.gen()),
        }
    }
}

impl Distribution<ConsensusInputMessage> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ConsensusInputMessage {
        ConsensusInputMessage {
            message: rng.gen(),
            recipient: rng.gen(),
        }
    }
}

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
    pub(crate) net: Arc<Network>,
    /// Termination signal that can be sent to the node.
    pub(crate) terminate: channel::Sender<()>,
    /// Dispatcher end of the network pipe.
    pub(crate) pipe: pipe::DispatcherPipe<crate::io::InputMessage, crate::io::OutputMessage>,
}

/// Construct configs for `n` validators of the consensus.
pub fn new_configs(
    rng: &mut impl Rng,
    setup: &validator::testonly::Setup,
    gossip_peers: usize,
) -> Vec<Config> {
    new_configs_for_validators(rng, setup.validator_keys.iter(), gossip_peers)
}

/// Construct configs for `n` validators of the consensus.
///
/// This version allows for repeating keys used in Twins tests.
pub fn new_configs_for_validators<'a, I>(
    rng: &mut impl Rng,
    validator_keys: I,
    gossip_peers: usize,
) -> Vec<Config>
where
    I: Iterator<Item = &'a validator::SecretKey>,
{
    let configs = validator_keys.map(|validator_key| {
        let addr = net::tcp::testonly::reserve_listener();
        Config {
            server_addr: addr,
            public_addr: (*addr).into(),
            // Pings are disabled in tests by default to avoid dropping connections
            // due to timeouts.
            ping_timeout: None,
            validator_key: Some(validator_key.clone()),
            gossip: GossipConfig {
                key: rng.gen(),
                dynamic_inbound_limit: usize::MAX,
                static_inbound: HashSet::default(),
                static_outbound: HashMap::default(),
            },
            max_block_size: usize::MAX,
            max_batch_size: usize::MAX,
            tcp_accept_rate: limiter::Rate::INF,
            rpc: RpcConfig::default(),
            max_block_queue_size: 10,
        }
    });
    let mut cfgs: Vec<_> = configs.collect();

    let n = cfgs.len();
    for i in 0..n {
        for j in 0..gossip_peers {
            let j = (i + j + 1) % n;
            let peer = cfgs[j].gossip.key.public();
            let addr = cfgs[j].public_addr.clone();
            cfgs[i].gossip.static_outbound.insert(peer, addr);
        }
    }
    cfgs
}

/// Constructs a config for a non-validator node, which will
/// establish a gossip connection to `peer`.
pub fn new_fullnode(rng: &mut impl Rng, peer: &Config) -> Config {
    let addr = net::tcp::testonly::reserve_listener();
    Config {
        server_addr: addr,
        public_addr: (*addr).into(),
        // Pings are disabled in tests by default to avoid dropping connections
        // due to timeouts.
        ping_timeout: None,
        validator_key: None,
        gossip: GossipConfig {
            key: rng.gen(),
            dynamic_inbound_limit: usize::MAX,
            static_inbound: HashSet::default(),
            static_outbound: [(peer.gossip.key.public(), peer.public_addr.clone())].into(),
        },
        max_block_size: usize::MAX,
        max_batch_size: usize::MAX,
        tcp_accept_rate: limiter::Rate::INF,
        rpc: RpcConfig::default(),
        max_block_queue_size: 10,
    }
}

/// Runner for Instance.
pub struct InstanceRunner {
    net_runner: Runner,
    batch_store: Arc<BatchStore>,
    terminate: channel::Receiver<()>,
}

impl InstanceRunner {
    /// Runs the instance background processes.
    pub async fn run(mut self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        scope::run!(ctx, |ctx, s| async {
            s.spawn_bg(self.net_runner.run(ctx));
            let _ = self.terminate.recv(ctx).await;
            Ok(())
        })
        .await?;
        drop(self.terminate);
        Ok(())
    }
}

/// InstanceConfig
pub struct InstanceConfig {
    /// cfg
    pub cfg: Config,
    /// block_store
    pub block_store: Arc<BlockStore>,
    /// batch_store
    pub batch_store: Arc<BatchStore>,
    /// attestation_state
    pub attestation_state: Arc<attestation::StateWatch>,
}

impl Instance {
    /// Constructs a new instance.
    pub fn new(
        cfg: Config,
        block_store: Arc<BlockStore>,
        batch_store: Arc<BatchStore>,
    ) -> (Self, InstanceRunner) {
        Self::new_from_config(InstanceConfig {
            cfg,
            block_store,
            batch_store,
            attestation_state: attestation::StateWatch::new(None).into(),
        })
    }

    /// Construct an instance for a given config.
    pub fn new_from_config(cfg: InstanceConfig) -> (Self, InstanceRunner) {
        let (actor_pipe, dispatcher_pipe) = pipe::new();
        let (net, net_runner) = Network::new(
            cfg.cfg,
            cfg.block_store.clone(),
            cfg.batch_store.clone(),
            actor_pipe,
            cfg.attestation_state,
        );
        let (terminate_send, terminate_recv) = channel::bounded(1);
        (
            Self {
                net,
                pipe: dispatcher_pipe,
                terminate: terminate_send,
            },
            InstanceRunner {
                net_runner,
                batch_store: cfg.batch_store.clone(),
                terminate: terminate_recv,
            },
        )
    }

    /// Sends a termination signal to a node
    /// and waits for it to terminate.
    pub async fn terminate(&self, ctx: &ctx::Ctx) -> ctx::OrCanceled<()> {
        let _ = self.terminate.try_send(());
        self.terminate.closed(ctx).await
    }

    /// Pipe getter.
    pub fn pipe(
        &mut self,
    ) -> &mut pipe::DispatcherPipe<crate::io::InputMessage, crate::io::OutputMessage> {
        &mut self.pipe
    }

    /// State getter.
    pub fn state(&self) -> &Arc<Network> {
        &self.net
    }

    /// Genesis.
    pub fn genesis(&self) -> &validator::Genesis {
        self.net.gossip.genesis()
    }

    /// Returns the gossip config for this node.
    pub fn cfg(&self) -> &Config {
        &self.net.gossip.cfg
    }

    /// Wait for static outbound gossip connections to be established.
    pub async fn wait_for_gossip_connections(&self) {
        let want: HashSet<_> = self.cfg().gossip.static_outbound.keys().cloned().collect();
        self.net
            .gossip
            .outbound
            .subscribe()
            .wait_for(|got| want.iter().all(|k| got.current().contains_key(k)))
            .await
            .unwrap();
    }

    /// Waits for all the consensus connections to be established.
    pub async fn wait_for_consensus_connections(&self) {
        let consensus_state = self.net.consensus.as_ref().unwrap();

        let want: HashSet<_> = self.genesis().validators.keys().cloned().collect();
        consensus_state
            .inbound
            .subscribe()
            .wait_for(|got| want.iter().all(|k| got.current().contains_key(k)))
            .await
            .unwrap();
        consensus_state
            .outbound
            .subscribe()
            .wait_for(|got| want.iter().all(|k| got.current().contains_key(k)))
            .await
            .unwrap();
    }

    /// Waits for node to be disconnected from the given gossip peer.
    pub async fn wait_for_gossip_disconnect(
        &self,
        ctx: &ctx::Ctx,
        peer: &node::PublicKey,
    ) -> ctx::OrCanceled<()> {
        let state = &self.net.gossip;
        sync::wait_for(ctx, &mut state.inbound.subscribe(), |got| {
            !got.current().contains_key(peer)
        })
        .await?;
        sync::wait_for(ctx, &mut state.outbound.subscribe(), |got| {
            !got.current().contains_key(peer)
        })
        .await?;
        Ok(())
    }

    /// Waits for node to be disconnected from the given consensus peer.
    pub async fn wait_for_consensus_disconnect(
        &self,
        ctx: &ctx::Ctx,
        peer: &validator::PublicKey,
    ) -> ctx::OrCanceled<()> {
        let state = self.net.consensus.as_ref().unwrap();
        sync::wait_for(ctx, &mut state.inbound.subscribe(), |got| {
            !got.current().contains_key(peer)
        })
        .await?;
        sync::wait_for(ctx, &mut state.outbound.subscribe(), |got| {
            !got.current().contains_key(peer)
        })
        .await?;
        Ok(())
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
        let key = node.cfg().validator_key.as_ref().unwrap().public();
        let sub = &mut node.net.gossip.validator_addrs.subscribe();
        loop {
            if let Some(addr) = sync::changed(ctx, sub).await?.get(&key) {
                addrs.push(addr.clone());
                break;
            }
        }
    }
    // Broadcast validator addrs.
    for node in &nodes {
        node.net
            .gossip
            .validator_addrs
            .update(&node.genesis().validators, &addrs)
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
