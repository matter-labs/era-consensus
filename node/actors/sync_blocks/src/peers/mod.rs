//! Peer states tracked by the `SyncBlocks` actor.

use self::events::PeerStateEvent;
use crate::{io, Config};
use anyhow::Context as _;
use std::{collections::HashMap, sync::Arc, sync::Mutex};
use zksync_concurrency::{
    ctx::{self, channel},
    oneshot, scope,
    sync,
};
use zksync_consensus_network::io::{SyncBlocksInputMessage};
use zksync_consensus_roles::{
    node,
    validator::{BlockNumber, FinalBlock},
};
use zksync_consensus_storage::{BlockStore,BlockStoreState};

mod events;
#[cfg(test)]
mod tests;

#[derive(Debug)]
struct PeerState {
    state: BlockStoreState,
    get_block_semaphore: Arc<sync::Semaphore>,
}

/// Handle for [`PeerStates`] allowing to send updates to it.
#[derive(Debug, Clone)]
pub(crate) struct PeerStates {
    config: Config,
    storage: Arc<BlockStore>,
    message_sender: channel::UnboundedSender<io::OutputMessage>,
    
    peers: Mutex<HashMap<node::PublicKey, PeerState>>,
    highest: sync::watch::Sender<BlockNumber>,
    events_sender: Option<channel::UnboundedSender<PeerStateEvent>>,
}

impl PeerStates {
    /// Creates a new instance together with a handle.
    pub(crate) fn new(
        config: Config,
        storage: Arc<BlockStore>,
        message_sender: channel::UnboundedSender<io::OutputMessage>,
    ) -> Self {
        Self{
            config,
            peers: Mutex::default(),
            highest: sync::watch::channel(BlockNumber(0)).0,
            events_sender: None,
        }
    }

    pub(crate) fn update(&self, peer: &node::PublicKey, state: BlockStoreState) -> anyhow::Result<()> {
        let last = state.last.header().number;
        let res = self.update_inner(peer, state);
        if let Err(err) = res {
            tracing::warn!(%err, %peer, "Invalid `SyncState` received");
            // TODO(gprusak): close the connection and ban.
        }
        if let Some(events_sender) = &self.events_sender {
            events_sender.send(match res {
                Ok(()) => PeerStateEvent::PeerUpdated(peer.clone()),
                Err(_) =>PeerStateEvent::InvalidPeerUpdate(peer.clone()),
            });
        }
        self.highest.send_if_modified(|highest| {
            if highest >= last {
                return false;
            }
            *highest = last;
            return true;
        });
    }

    /// Returns the last trusted block number stored by the peer.
    async fn update_inner(&self, peer: &node::PublicKey, state: BlockStoreState) -> anyhow::Result<()> {
        anyhow::ensure!(state.first.header().number <= state.last.header().number);
        state
            .last
            .verify(&self.config.validator_set, self.config.consensus_threshold)
            .context("state.last.verify()");
        let mut peers = self.peers.lock().unwrap();
        let permits = self.config.max_concurrent_blocks_per_peer;
        let peer_state = peers.entry(peer.clone()).or_insert_with(|| PeerState {
            state,
            get_block_semaphore: Arc::new(sync::Semaphore::new(permits)),
        });
        peer_state.state = state;
        Ok(())
    }

    /// Task fetching blocks from peers which are not present in storage.
    pub(crate) async fn run_block_fetcher(&self, ctx: &ctx::Ctx) {
        let sem = sync::Semaphore::new(self.config.max_concurrent_blocks);
        // Ignore cancellation error.
        let _ = scope::run!(ctx, |ctx, s| async {
            let mut next = self.storage.subscribe().borrow().next();
            let mut highest = self.highest.subscribe();
            loop {
                *sync::wait_for(ctx, &mut highest, |highest| highest >= &next).await?;
                let permit = sync::acquire(ctx, &sem).await?;
                s.spawn(async {
                    let permit = permit;
                    self.fetch_block(ctx, next).await
                });
                next = next.next();
            }
        }).await;
    }

    /// Fetches the block from peers and puts it to storage.
    /// Early exits if the block appeared in storage from other source.
    async fn fetch_block(&self, ctx: &ctx::Ctx, block_number: BlockNumber) -> ctx::OrCanceled<()> {
        scope::run!(ctx, |ctx,s| async {
            s.spawn_bg(async {
                let res = self.fetch_block_from_peers(ctx,block_number).await;
                if let Some(events_sender) = &self.events_sender {
                    events_sender.send(match res {
                        Ok(()) => PeerStateEvent::GotBlock(block_number),
                        Err(ctx::Canceled) => PeerStateEvent::CanceledBlock(block_number),
                    });
                }
                Ok(())
            });
            // Observe if the block has appeared in storage.
            sync::wait_for(ctx, &mut self.storage.subscribe(), |state| state.next() > block_number).await
        }).await
    }

    /// Fetches the block from peers and puts it to storage.
    async fn fetch_block_from_peers(
        &self,
        ctx: &ctx::Ctx,
        number: BlockNumber,
    ) -> ctx::OrCanceled<()> {
        loop { 
            let Some((peer, _permit)) = self.try_acquire_peer_permit(number) else {
                let sleep_interval = self.config.sleep_interval_for_get_block;
                ctx.sleep(sleep_interval).await?;
                continue;
            };
            match self.fetch_block_from_peer(ctx, &peer, number).await {
                Ok(block) => return self.storage.queue_block(ctx, &block).await,
                Err(err) => {
                    tracing::info!(%err, "get_block({peer},{number}) failed, dropping peer");
                    if let Some(events_sender) = &self.events_sender {
                        events_sender.send(PeerStateEvent::RpcFailed { peer_key: peer.clone(), block_number: number });
                    }
                    self.forget_peer(&peer);
                }
            }
        }
    }
    
    /// Fetches a block from the specified peer.
    async fn fetch_block_from_peer(
        &self,
        ctx: &ctx::Ctx,
        peer: &node::PublicKey,
        number: BlockNumber,
    ) -> ctx::Result<FinalBlock> {
        let (response, response_receiver) = oneshot::channel();
        let message = SyncBlocksInputMessage::GetBlock {
            recipient: peer.clone(),
            number,
            response,
        };
        self.message_sender.send(message.into());
        let block = response_receiver.recv_or_disconnected(ctx)
            .await?
            .context("no response")?
            .context("RPC error")?; 
        if block.header().number != number {
            return Err(anyhow::anyhow!(
                "block does not have requested number (requested: {number}, got: {})",
                block.header().number
            ).into());
        }
        block.validate(&self.config.validator_set, self.config.consensus_threshold)
            .context("block.validate()")?;
        Ok(block)
    }

    fn try_acquire_peer_permit(&self, block_number: BlockNumber) -> Option<(node::PublicKey, sync::OwnedSemaphorePermit)> {
        let peers = self.peers.lock().unwrap();
        let mut peers_with_no_permits = vec![];
        let eligible_peers_info = peers.iter().filter(|(peer_key, state)| {
            if !state.has_block(block_number) {
                return false;
            }
            let available_permits = state.get_block_semaphore.available_permits();
            // ^ `available_permits()` provides a lower bound on the actual number of available permits.
            // Some permits may be released before acquiring a new permit below, but no other permits
            // are acquired since we hold an exclusive lock on `peers`.
            if available_permits == 0 {
                peers_with_no_permits.push(*peer_key);
            }
            available_permits > 0
        });
        let peer_to_query = eligible_peers_info
            .max_by_key(|(_, state)| state.get_block_semaphore.available_permits());

        if let Some((peer_key, state)) = peer_to_query {
            let permit = state
                .get_block_semaphore
                .clone()
                .try_acquire_owned()
                .unwrap();
            // ^ `unwrap()` is safe for the reasons described in the above comment
            Some((peer_key.clone(), permit))
        } else {
            tracing::debug!(
                %block_number,
                ?peers_with_no_permits,
                "No peers to query block #{block_number}"
            );
            None
        }
    }

    /// Drops peer state.
    async fn drop_peer(&self, peer: &node::PublicKey) {
        if self.peers.lock().unwrap().remove(peer).is_none() { return }
        tracing::trace!(?peer, "Dropping peer state");
        if let Some(events_sender) = &self.events_sender {
            events_sender.send(PeerStateEvent::PeerDropped(peer.clone()));
        }
    }
}
