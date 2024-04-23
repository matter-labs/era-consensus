//! Gossip network is a sparse graph of connections between nodes (not necessarily validators).
//! It will be used for:
//! * discovery of validators (consensus network will be bootstrapped from data received over
//!   the gossip network).
//! * broadcasting the finalized blocks
//! * P2P block synchronization (for non-validators)
//!
//! Gossip network consists of
//! * static connections (explicitly declared in configs of both ends of the connection).
//! * dynamic connections (additional randomized connections which are established to improve
//!   the throughput of the network).
//! Static connections constitute a rigid "backbone" of the gossip network, which is insensitive to
//! eclipse attack. Dynamic connections are supposed to improve the properties of the gossip
//! network graph (minimize its diameter, increase connectedness).
use crate::{gossip::ValidatorAddrsWatch, io, pool::PoolWatch, rpc, Config};
use anyhow::Context as _;
use std::sync::{atomic::AtomicUsize, Arc};

mod handshake;
mod runner;
mod get_block;
#[cfg(test)]
mod tests;
mod validator_addrs;

pub(crate) use validator_addrs::*;
use zksync_concurrency::{ctx, ctx::channel};
use zksync_consensus_roles::{node, validator};
use zksync_consensus_storage::BlockStore;
use zksync_protobuf::kB;

/// Gossip network state.
pub(crate) struct Network {
    /// Gossip network configuration.
    pub(crate) cfg: Config,
    /// Currently open inbound connections.
    pub(crate) inbound: PoolWatch<node::PublicKey, ()>,
    /// Currently open outbound connections.
    pub(crate) outbound: PoolWatch<node::PublicKey, ()>,
    /// Current state of knowledge about validators' endpoints.
    pub(crate) validator_addrs: ValidatorAddrsWatch,
    /// Block store to serve `get_block` requests from.
    pub(crate) block_store: Arc<BlockStore>,
    /// Output pipe of the network actor.
    pub(crate) sender: channel::UnboundedSender<io::OutputMessage>,
    /// Queue of `get_block` calls.
    pub(crate) get_block_queue: get_block::Queue, 
    /// TESTONLY: how many time push_validator_addrs rpc was called by the peers.
    pub(crate) push_validator_addrs_calls: AtomicUsize,
}

impl Network {
    /// Constructs a new State.
    pub(crate) fn new(
        cfg: Config,
        block_store: Arc<BlockStore>,
        sender: channel::UnboundedSender<io::OutputMessage>,
    ) -> Arc<Self> {
        Arc::new(Self {
            sender,
            inbound: PoolWatch::new(
                cfg.gossip.static_inbound.clone(),
                cfg.gossip.dynamic_inbound_limit,
            ),
            outbound: PoolWatch::new(cfg.gossip.static_outbound.keys().cloned().collect(), 0),
            validator_addrs: ValidatorAddrsWatch::default(),
            block_store,
            cfg,
            push_validator_addrs_calls: 0.into(),
        })
    }

    /// Genesis.
    pub(crate) fn genesis(&self) -> &validator::Genesis {
        self.block_store.genesis()
    }

    /// Sends a GetBlock RPC to the given peer.
    pub(crate) async fn get_block(
        &self,
        ctx: &ctx::Ctx,
        recipient: &node::PublicKey,
        number: validator::BlockNumber,
    ) -> anyhow::Result<Option<validator::FinalBlock>> {
        let outbound = self.outbound.current();
        let inbound = self.inbound.current();
        Ok(outbound
            .get(recipient)
            .or(inbound.get(recipient))
            .context("recipient is unreachable")?
            .get_block
            .call(
                ctx,
                &rpc::get_block::Req(number),
                self.cfg.max_block_size.saturating_add(kB),
            )
            .await?
            .0)
    }
}
