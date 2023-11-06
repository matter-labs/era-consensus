//! Peer states tracked by the `SyncBlocks` actor.

use self::events::PeerStateEvent;
use crate::{io, Config};
use anyhow::Context as _;
use std::{collections::HashMap, sync::Arc};
use tracing::instrument;
use zksync_concurrency::{
    ctx::{self, channel},
    oneshot, scope,
    sync::{self, watch, Mutex, Semaphore},
};
use zksync_consensus_network::io::{SyncBlocksInputMessage, SyncState};
use zksync_consensus_roles::{
    node,
    validator::{BlockHeader, BlockNumber, FinalBlock, PayloadHash},
};
use zksync_consensus_storage::WriteBlockStore;

mod events;
#[cfg(test)]
mod tests;

type PeerStateUpdate = (node::PublicKey, SyncState);

#[derive(Debug)]
struct PeerState {
    last_contiguous_stored_block: BlockNumber,
    get_block_semaphore: Arc<Semaphore>,
}

/// Handle for [`PeerStates`] allowing to send updates to it.
#[derive(Debug, Clone)]
pub(crate) struct PeerStatesHandle {
    updates_sender: channel::UnboundedSender<PeerStateUpdate>,
}

impl PeerStatesHandle {
    /// Notifies [`PeerStates`] about an updated [`SyncState`] of a peer.
    pub(crate) fn update(&self, peer_key: node::PublicKey, sync_state: SyncState) {
        self.updates_sender.send((peer_key, sync_state));
    }
}

/// View of peers (or more precisely, connections with peers) w.r.t. block syncing.
#[derive(Debug)]
pub(crate) struct PeerStates {
    updates_receiver: Option<channel::UnboundedReceiver<PeerStateUpdate>>,
    events_sender: Option<channel::UnboundedSender<PeerStateEvent>>,
    peers: Mutex<HashMap<node::PublicKey, PeerState>>,
    message_sender: channel::UnboundedSender<io::OutputMessage>,
    storage: Arc<dyn WriteBlockStore>,
    config: Config,
}

impl PeerStates {
    /// Creates a new instance together with a handle.
    pub(crate) fn new(
        message_sender: channel::UnboundedSender<io::OutputMessage>,
        storage: Arc<dyn WriteBlockStore>,
        config: Config,
    ) -> (Self, PeerStatesHandle) {
        let (updates_sender, updates_receiver) = channel::unbounded();
        let this = Self {
            updates_receiver: Some(updates_receiver),
            events_sender: None,
            peers: Mutex::default(),
            message_sender,
            storage,
            config,
        };
        let handle = PeerStatesHandle { updates_sender };
        (this, handle)
    }

    /// Runs the sub-actor. This will:
    ///
    /// 1. Get information about missing blocks from the storage.
    /// 2. Spawn a task processing `SyncState`s from peers.
    /// 3. Spawn a task to get each missing block.
    pub(crate) async fn run(mut self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        let updates_receiver = self.updates_receiver.take().unwrap();
        let storage = self.storage.as_ref();
        let get_block_semaphore = Semaphore::new(self.config.max_concurrent_blocks);
        let (new_blocks_sender, mut new_blocks_subscriber) = watch::channel(BlockNumber(0));

        scope::run!(ctx, |ctx, s| async {
            let start_number = storage.last_contiguous_block_number(ctx).await?;
            let mut last_block_number = storage.head_block(ctx).await?.header.number;
            let missing_blocks = storage
                .missing_block_numbers(ctx, start_number..last_block_number)
                .await?;
            new_blocks_sender.send_replace(last_block_number);

            s.spawn_bg(self.run_updates(ctx, updates_receiver, new_blocks_sender));

            for block_number in missing_blocks {
                let get_block_permit = sync::acquire(ctx, &get_block_semaphore).await?;
                s.spawn(self.get_and_save_block(ctx, block_number, get_block_permit, storage));
            }

            loop {
                let new_last_block_number = *sync::changed(ctx, &mut new_blocks_subscriber).await?;
                let new_block_numbers =
                    (last_block_number.next().0..=new_last_block_number.0).map(BlockNumber);
                tracing::trace!(
                    ?new_block_numbers,
                    "Enqueuing requests for getting blocks from peers"
                );

                for block_number in new_block_numbers {
                    let get_block_permit = sync::acquire(ctx, &get_block_semaphore).await?;
                    s.spawn(self.get_and_save_block(ctx, block_number, get_block_permit, storage));
                }
                last_block_number = new_last_block_number;
            }
        })
        .await
    }

    async fn run_updates(
        &self,
        ctx: &ctx::Ctx,
        mut updates_receiver: channel::UnboundedReceiver<PeerStateUpdate>,
        new_blocks_sender: watch::Sender<BlockNumber>,
    ) -> anyhow::Result<()> {
        loop {
            let (peer_key, sync_state) = updates_receiver.recv(ctx).await?;
            let new_last_block_number = self
                .update_peer_sync_state(ctx, peer_key, sync_state)
                .await?;
            new_blocks_sender.send_if_modified(|number| {
                if *number < new_last_block_number {
                    *number = new_last_block_number;
                    return true;
                }
                false
            });
        }
    }

    /// Returns the last trusted block number stored by the peer.
    #[instrument(
        level = "trace",
        err,
        skip(self, ctx, state),
        fields(state = ?state.numbers())
    )]
    async fn update_peer_sync_state(
        &self,
        ctx: &ctx::Ctx,
        peer_key: node::PublicKey,
        state: SyncState,
    ) -> anyhow::Result<BlockNumber> {
        let last_contiguous_stored_block = match self.validate_sync_state(state) {
            Ok(block_number) => block_number,
            Err(err) => {
                tracing::warn!(%err, "Invalid `SyncState` received from peer");
                if let Some(events_sender) = &self.events_sender {
                    events_sender.send(PeerStateEvent::InvalidPeerUpdate(peer_key));
                }
                return Ok(BlockNumber(0));
                // TODO: ban peer etc.
            }
        };

        let mut peers = sync::lock(ctx, &self.peers).await?;
        let permits = self.config.max_concurrent_blocks_per_peer;
        let peer_state = peers.entry(peer_key.clone()).or_insert_with(|| PeerState {
            last_contiguous_stored_block,
            get_block_semaphore: Arc::new(Semaphore::new(permits)),
        });
        let prev_contiguous_stored_block = peer_state.last_contiguous_stored_block;
        if last_contiguous_stored_block < prev_contiguous_stored_block {
            tracing::warn!(
                %last_contiguous_stored_block,
                %prev_contiguous_stored_block,
                "Bogus state update from peer: new `last_contiguous_stored_block` value \
                 ({last_contiguous_stored_block}) is lesser than the old value ({prev_contiguous_stored_block})"
            );
        }
        tracing::trace!(
            %prev_contiguous_stored_block,
            %last_contiguous_stored_block,
            "Updating last contiguous stored block for peer"
        );
        peer_state.last_contiguous_stored_block = last_contiguous_stored_block;
        drop(peers);

        if let Some(events_sender) = &self.events_sender {
            events_sender.send(PeerStateEvent::PeerUpdated(peer_key));
        }
        Ok(last_contiguous_stored_block)
    }

    fn validate_sync_state(&self, state: SyncState) -> anyhow::Result<BlockNumber> {
        let numbers = state.numbers();
        anyhow::ensure!(
            numbers.first_stored_block <= numbers.last_contiguous_stored_block,
            "Invariant violated: numbers.first_stored_block <= numbers.last_contiguous_stored_block"
        );
        anyhow::ensure!(
            numbers.last_contiguous_stored_block <= numbers.last_stored_block,
            "Invariant violated: numbers.last_contiguous_stored_block <= numbers.last_stored_block"
        );

        state
            .last_contiguous_stored_block
            .verify(&self.config.validator_set, self.config.consensus_threshold)
            .context("Failed verifying `last_contiguous_stored_block`")?;
        // We don't verify QCs for first and last stored blocks since they are not used
        // in the following logic. To reflect this, the method consumes `SyncState` and returns
        // the validated block number.
        Ok(numbers.last_contiguous_stored_block)
    }

    async fn get_and_save_block(
        &self,
        ctx: &ctx::Ctx,
        block_number: BlockNumber,
        get_block_permit: sync::SemaphorePermit<'_>,
        storage: &dyn WriteBlockStore,
    ) -> anyhow::Result<()> {
        let block = self.get_block(ctx, block_number).await?;
        drop(get_block_permit);

        if let Some(events_sender) = &self.events_sender {
            events_sender.send(PeerStateEvent::GotBlock(block_number));
        }
        storage.put_block(ctx, &block).await?;
        Ok(())
    }

    #[instrument(level = "trace", skip(self, ctx))]
    async fn get_block(
        &self,
        ctx: &ctx::Ctx,
        block_number: BlockNumber,
    ) -> anyhow::Result<FinalBlock> {
        loop {
            let Some((peer_key, _permit)) =
                Self::acquire_peer_permit(&*sync::lock(ctx, &self.peers).await?, block_number)
            else {
                let sleep_interval = self.config.sleep_interval_for_get_block;
                ctx.sleep(sleep_interval).await?;
                continue;
            };

            let block = self
                .get_block_from_peer(ctx, peer_key.clone(), block_number)
                .await?;
            let Some(block) = block else { continue };

            if let Err(err) = self.validate_block(block_number, &block) {
                tracing::warn!(
                    %err, ?peer_key, %block_number,
                    "Received invalid block #{block_number} from peer {peer_key:?}"
                );
                // TODO: ban peer etc.
                if let Some(events_sender) = &self.events_sender {
                    events_sender.send(PeerStateEvent::GotInvalidBlock {
                        peer_key,
                        block_number,
                    });
                }
            } else {
                return Ok(block);
            }
        }
    }

    // It's important to keep this method sync; we don't want to hold `peers` lock across wait points.
    fn acquire_peer_permit(
        peers: &HashMap<node::PublicKey, PeerState>,
        block_number: BlockNumber,
    ) -> Option<(node::PublicKey, sync::OwnedSemaphorePermit)> {
        let mut peers_with_no_permits = vec![];
        let eligible_peers_info = peers.iter().filter(|(peer_key, state)| {
            if state.last_contiguous_stored_block < block_number {
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

    #[instrument(level = "trace", skip(self, ctx), err)]
    async fn get_block_from_peer(
        &self,
        ctx: &ctx::Ctx,
        recipient: node::PublicKey,
        number: BlockNumber,
    ) -> anyhow::Result<Option<FinalBlock>> {
        let (response, response_receiver) = oneshot::channel();
        let message = SyncBlocksInputMessage::GetBlock {
            recipient: recipient.clone(),
            number,
            response,
        };
        self.message_sender.send(message.into());
        tracing::trace!("Requested block from peer");

        let response = response_receiver.recv_or_disconnected(ctx).await?;
        match response {
            Ok(Ok(block)) => return Ok(Some(block)),
            Ok(Err(rpc_err)) => {
                tracing::warn!(
                    err = %rpc_err,
                    "get_block({number}) returned an error"
                );
            }
            Err(_) => {
                tracing::info!("get_block({number}) request was dropped by network");
                self.disconnect_peer(ctx, &recipient).await?;
            }
        }
        Ok(None)
    }

    fn validate_block(
        &self,
        block_number: BlockNumber,
        block: &FinalBlock,
    ) -> Result<(), BlockValidationError> {
        if block.header.number != block_number {
            return Err(BlockValidationError::NumberMismatch {
                requested: block_number,
                got: block.header.number,
            });
        }
        let payload_hash = block.payload.hash();
        if payload_hash != block.header.payload {
            return Err(BlockValidationError::HashMismatch {
                header_hash: block.header.payload,
                payload_hash,
            });
        }
        if block.header != block.justification.message.proposal {
            return Err(BlockValidationError::ProposalMismatch {
                block_header: Box::new(block.header),
                qc_header: Box::new(block.justification.message.proposal),
            });
        }

        block
            .justification
            .verify(&self.config.validator_set, self.config.consensus_threshold)
            .map_err(BlockValidationError::Justification)
    }

    #[instrument(level = "trace", skip(self, ctx))]
    async fn disconnect_peer(
        &self,
        ctx: &ctx::Ctx,
        peer_key: &node::PublicKey,
    ) -> anyhow::Result<()> {
        let mut peers = sync::lock(ctx, &self.peers).await?;
        if let Some(state) = peers.remove(peer_key) {
            tracing::trace!(?state, "Dropping peer connection state");
        }
        if let Some(events_sender) = &self.events_sender {
            events_sender.send(PeerStateEvent::PeerDisconnected(peer_key.clone()));
        }
        Ok(())
    }
}

/// Errors that can occur validating a `FinalBlock` received from a node.
#[derive(Debug, thiserror::Error)]
enum BlockValidationError {
    #[error("block does not have requested number (requested: {requested}, got: {got})")]
    NumberMismatch {
        requested: BlockNumber,
        got: BlockNumber,
    },
    #[error(
        "block payload doesn't match the block header (hash in header: {header_hash:?}, \
         payload hash: {payload_hash:?})"
    )]
    HashMismatch {
        header_hash: PayloadHash,
        payload_hash: PayloadHash,
    },
    #[error(
        "quorum certificate proposal doesn't match the block header (block header: {block_header:?}, \
         header in QC: {qc_header:?})"
    )]
    ProposalMismatch {
        block_header: Box<BlockHeader>,
        qc_header: Box<BlockHeader>,
    },
    #[error("failed verifying quorum certificate: {0:#?}")]
    Justification(#[source] anyhow::Error),
}
