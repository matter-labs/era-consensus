//! Testonly utilities.
#![allow(dead_code)]
use crate::{consensus, event::Event, gossip, Config, State};
use rand::Rng;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use zksync_consensus_storage::{BlockStore,BlockStoreRunner,testonly::in_memory};
use zksync_concurrency::{
    signal,
    ctx,
    ctx::channel,
    scope,
    io, net,
    sync::{self},
};
use zksync_consensus_roles::{validator,node};
use zksync_consensus_utils::pipe;

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
    pub(crate) terminate: Arc<signal::Once>,
    /// Dispatcher end of the network pipe.
    pub pipe: pipe::DispatcherPipe<crate::io::InputMessage, crate::io::OutputMessage>,
}

/// Constructs a new store with a genesis block.
pub async fn new_store(ctx: &ctx::Ctx, genesis: &validator::FinalBlock) -> (Arc<BlockStore>,BlockStoreRunner) {
    BlockStore::new(ctx,Box::new(in_memory::BlockStore::new(genesis.clone()))).await.unwrap()
}

/// Construct configs for `n` validators of the consensus.
pub fn new_configs<R: Rng>(rng: &mut R, setup: &validator::testonly::GenesisSetup, gossip_peers: usize) -> Vec<Config> {
    let configs = setup.keys.iter().map(|key| {
        let addr = net::tcp::testonly::reserve_listener();
        Config {
            server_addr: addr,
            validators: setup.validator_set(),
            // Pings are disabled in tests by default to avoid dropping connections
            // due to timeouts.
            enable_pings: false,
            consensus: Some(consensus::Config {
                key: key.clone(),
                public_addr: *addr,
            }),
            gossip: gossip::Config {
                key: rng.gen(),
                dynamic_inbound_limit: setup.keys.len() as u64,
                static_inbound: HashSet::default(),
                static_outbound: HashMap::default(), 
            },
        }
    });
    let mut cfgs: Vec<_> = configs.collect();

    let n = cfgs.len();
    for i in 0..n {
        for j in 0..gossip_peers {
            let j = (i + j + 1) % n;
            let peer = cfgs[j].gossip.key.public();
            let addr = *cfgs[j].server_addr;
            cfgs[i].gossip.static_outbound.insert(peer, addr);
        }
    }
    cfgs
}

/// Runner for Instance.
pub struct InstanceRunner {
    state: Arc<State>,
    terminate: Arc<signal::Once>,
    pipe: pipe::ActorPipe<crate::io::InputMessage, crate::io::OutputMessage>,
}

impl InstanceRunner {
    /// Runs the instance background processes.
    pub async fn run(self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        scope::run!(ctx, |ctx,s| async {
            s.spawn_bg(async {
                crate::run_network(ctx, self.state, self.pipe).await?;
                Ok(())
            }); 
            let _ = self.terminate.recv(ctx).await;
            Ok(())
        }).await
    }  
}

impl Instance {
    /// Construct an instance for a given config.
    pub fn new(cfg: Config, block_store: Arc<BlockStore>) -> (Self,InstanceRunner) {
        let (events_send, events_recv) = channel::unbounded();
        let (actor_pipe, dispatcher_pipe) = pipe::new();
        let state = State::new(cfg, block_store, Some(events_send)).expect("Invalid network config");
        let terminate = Arc::new(signal::Once::new());
        (Self {
            state: state.clone(), 
            events: events_recv,
            pipe: dispatcher_pipe,
            terminate: terminate.clone(),
        }, InstanceRunner {
            state: state.clone(),
            pipe: actor_pipe,
            terminate,
        })
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

    /// Waits for node to be disconnected from the given gossip peer.
    pub async fn wait_for_gossip_disconnect(&self, ctx: &ctx::Ctx, peer: &node::PublicKey) -> ctx::OrCanceled<()> {
        let state = &self.state.gossip;
        sync::wait_for(ctx, &mut state.inbound.subscribe(), |got| !got.current().contains(peer)).await?;
        sync::wait_for(ctx, &mut state.outbound.subscribe(), |got| !got.current().contains(peer)).await?;
        Ok(())
    }

    /// Waits for node to be disconnected from the given consensus peer.
    pub async fn wait_for_consensus_disconnect(&self, ctx: &ctx::Ctx, peer: &validator::PublicKey) -> ctx::OrCanceled<()> {
        let state = self.state.consensus.as_ref().unwrap();
        sync::wait_for(ctx, &mut state.inbound.subscribe(), |got| !got.current().contains(peer)).await?;
        sync::wait_for(ctx, &mut state.outbound.subscribe(), |got| !got.current().contains(peer)).await?;
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
