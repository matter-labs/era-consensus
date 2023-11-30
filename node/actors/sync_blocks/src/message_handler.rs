//! Inner details of `SyncBlocks` actor.

use crate::{io::InputMessage, peers::PeerStatesHandle};
use std::sync::Arc;
use tracing::instrument;
use zksync_concurrency::ctx::{self, channel};
use zksync_consensus_network::io::{GetBlockError, GetBlockResponse, SyncBlocksRequest};
use zksync_consensus_roles::validator::BlockNumber;
use zksync_consensus_storage::WriteBlockStore;

/// Inner details of `SyncBlocks` actor allowing to process messages.
#[derive(Debug)]
pub(crate) struct SyncBlocksMessageHandler {
    /// Pipe using which the actor sends / receives messages.
    pub(crate) message_receiver: channel::UnboundedReceiver<InputMessage>,
    /// Persistent storage for blocks.
    pub(crate) storage: Arc<dyn WriteBlockStore>,
    /// Set of validators authoring blocks.
    pub(crate) peer_states_handle: PeerStatesHandle,
}

impl SyncBlocksMessageHandler {
    /// Implements the message processing loop.
    #[instrument(level = "trace", skip_all, err)]
    pub(crate) async fn process_messages(mut self, ctx: &ctx::Ctx) -> ctx::Result<()> {
        while let Ok(input_message) = self.message_receiver.recv(ctx).await {
            match input_message {
                InputMessage::Network(SyncBlocksRequest::UpdatePeerSyncState {
                    peer,
                    state,
                    response,
                }) => {
                    self.peer_states_handle.update(peer, *state);
                    response.send(()).ok();
                }
                InputMessage::Network(SyncBlocksRequest::GetBlock {
                    block_number,
                    response,
                }) => {
                    response.send(self.get_block(ctx, block_number).await?).ok();
                }
            }
        }
        Ok(())
    }

    /// Gets a block with the specified `number` from the storage.
    ///
    /// **This method is blocking.**
    #[instrument(level = "trace", skip(self, ctx), err)]
    async fn get_block(
        &self,
        ctx: &ctx::Ctx,
        number: BlockNumber,
    ) -> ctx::Result<GetBlockResponse> {
        Ok(self
            .storage
            .block(ctx, number)
            .await?
            .ok_or(GetBlockError::NotSynced))
    }
}
