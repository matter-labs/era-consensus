//! Peer states tracked by the `SyncBlocks` actor.

use self::events::PeerStateEvent;
use crate::{io, Config};
use anyhow::Context as _;
use std::{collections::HashMap, sync::Arc, sync::Mutex};
use zksync_concurrency::{
    ctx::{self, channel},
    oneshot, scope, sync,
};
use zksync_consensus_network::io::SyncBlocksInputMessage;
use zksync_consensus_roles::{
    node,
    validator::{BlockNumber, FinalBlock},
};
use zksync_consensus_storage::{BlockStore, BlockStoreState};
use zksync_consensus_utils::no_copy::NoCopy;

mod events;
#[cfg(test)]
mod tests;

#[derive(Debug)]
struct PeerState {
    state: BlockStoreState,
    get_block_semaphore: Arc<sync::Semaphore>,
}

/// Handle for [`PeerStates`] allowing to send updates to it.
#[derive(Debug)]
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
        Self {
            config,
            storage,
            message_sender,

            peers: Mutex::default(),
            highest: sync::watch::channel(BlockNumber(0)).0,
            events_sender: None,
        }
    }

    pub(crate) fn update(
        &self,
        peer: &node::PublicKey,
        state: BlockStoreState,
    ) -> anyhow::Result<()> {
        let last = state.last.header().number;
        anyhow::ensure!(state.first.header().number <= state.last.header().number);
        state
            .last
            .verify(&self.config.validator_set, self.config.consensus_threshold)
            .context("state.last.verify()")?;
        let mut peers = self.peers.lock().unwrap();
        let permits = self.config.max_concurrent_blocks_per_peer;
        use std::collections::hash_map::Entry;
        match peers.entry(peer.clone()) {
            Entry::Occupied(mut e) => e.get_mut().state = state,
            Entry::Vacant(e) => {
                e.insert(PeerState {
                    state,
                    get_block_semaphore: Arc::new(sync::Semaphore::new(permits)),
                });
            }
        }
        self.highest.send_if_modified(|highest| {
            if *highest >= last {
                return false;
            }
            *highest = last;
            return true;
        });
        Ok(())
    }

    /// Task fetching blocks from peers which are not present in storage.
    pub(crate) async fn run_block_fetcher(&self, ctx: &ctx::Ctx) -> ctx::OrCanceled<()> {
        let sem = sync::Semaphore::new(self.config.max_concurrent_blocks);
        scope::run!(ctx, |ctx, s| async {
            let mut next = self.storage.subscribe().borrow().next();
            let mut highest = self.highest.subscribe();
            loop {
                sync::wait_for(ctx, &mut highest, |highest| highest >= &next).await?;
                let permit = sync::acquire(ctx, &sem).await?;
                let block_number = NoCopy::from(next);
                next = next.next();
                s.spawn(async {
                    let _permit = permit;
                    self.fetch_block(ctx, block_number.into_inner()).await
                });
            }
        })
        .await
    }

    /// Fetches the block from peers and puts it to storage.
    /// Early exits if the block appeared in storage from other source.
    async fn fetch_block(&self, ctx: &ctx::Ctx, block_number: BlockNumber) -> ctx::OrCanceled<()> {
        scope::run!(ctx, |ctx, s| async {
            s.spawn_bg(async {
                if let Err(ctx::Canceled) = self.fetch_block_from_peers(ctx, block_number).await {
                    if let Some(send) = &self.events_sender {
                        send.send(PeerStateEvent::CanceledBlock(block_number));
                    }
                }
                Ok(())
            });
            // Observe if the block has appeared in storage.
            sync::wait_for(ctx, &mut self.storage.subscribe(), |state| {
                state.next() > block_number
            })
            .await?;
            Ok(())
        })
        .await
    }

    /// Fetches the block from peers and puts it to storage.
    async fn fetch_block_from_peers(
        &self,
        ctx: &ctx::Ctx,
        number: BlockNumber,
    ) -> ctx::OrCanceled<()> {
        while ctx.is_active() {
            let Some((peer, permit)) = self.try_acquire_peer_permit(number) else {
                let sleep_interval = self.config.sleep_interval_for_get_block;
                ctx.sleep(sleep_interval).await?;
                continue;
            };
            let res = self.fetch_block_from_peer(ctx, &peer, number).await;
            drop(permit);
            match res {
                Ok(block) => {
                    if let Some(send) = &self.events_sender {
                        send.send(PeerStateEvent::GotBlock(number));
                    }
                    return self.storage.queue_block(ctx, block).await;
                }
                Err(ctx::Error::Canceled(_)) => {
                    tracing::info!(%number, ?peer, "get_block() call canceled");
                }
                Err(err) => {
                    tracing::info!(%err, %number, ?peer, "get_block() failed");
                    if let Some(send) = &self.events_sender {
                        send.send(PeerStateEvent::RpcFailed {
                            peer_key: peer.clone(),
                            block_number: number,
                        });
                    }
                    self.drop_peer(&peer);
                }
            }
        }
        Err(ctx::Canceled.into())
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
        let block = response_receiver
            .recv_or_disconnected(ctx)
            .await?
            .context("no response")?
            .context("RPC error")?;
        if block.header().number != number {
            return Err(anyhow::anyhow!(
                "block does not have requested number (requested: {number}, got: {})",
                block.header().number
            )
            .into());
        }
        block
            .validate(&self.config.validator_set, self.config.consensus_threshold)
            .context("block.validate()")?;
        Ok(block)
    }

    fn try_acquire_peer_permit(
        &self,
        block_number: BlockNumber,
    ) -> Option<(node::PublicKey, sync::OwnedSemaphorePermit)> {
        let peers = self.peers.lock().unwrap();
        let mut peers_with_no_permits = vec![];
        let eligible_peers_info = peers.iter().filter(|(peer_key, state)| {
            if !state.state.contains(block_number) {
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
                ?peers_with_no_permits,
                "No peers to query block #{block_number}"
            );
            None
        }
    }

    /// Drops peer state.
    fn drop_peer(&self, peer: &node::PublicKey) {
        if self.peers.lock().unwrap().remove(peer).is_none() {
            return;
        }
        tracing::debug!(?peer, "Dropping peer state");
        if let Some(events_sender) = &self.events_sender {
            events_sender.send(PeerStateEvent::PeerDropped(peer.clone()));
        }
    }
}
