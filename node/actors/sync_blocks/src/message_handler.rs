//! Inner details of `SyncBlocks` actor.

use crate::{io::InputMessage, peers::PeerStatesHandle};
use concurrency::ctx::{self, channel};
use network::io::{GetBlockError, GetBlockResponse, SyncBlocksRequest};
use roles::validator::BlockNumber;
use std::sync::Arc;
use storage::Storage;
use tracing::instrument;

/// Inner details of `SyncBlocks` actor allowing to process messages.
#[derive(Debug)]
pub(crate) struct SyncBlocksMessageHandler {
    /// Pipe using which the actor sends / receives messages.
    pub(crate) message_receiver: channel::UnboundedReceiver<InputMessage>,
    /// Persistent storage for blocks.
    pub(crate) storage: Arc<Storage>,
    /// Set of validators authoring blocks.
    pub(crate) peer_states_handle: PeerStatesHandle,
}

impl SyncBlocksMessageHandler {
    /// Implements the message processing loop.
    ///
    /// **This method is blocking and will run indefinitely.**
    #[instrument(level = "trace", skip_all, err)]
    pub(crate) fn process_messages(mut self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        loop {
            let input_message = self.message_receiver.recv(ctx).block()?;
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
                    response.send(self.get_block(block_number)).ok();
                }
            }
        }
    }

    /// Gets a block with the specified `number` from the storage.
    ///
    /// **This method is blocking.**
    #[instrument(level = "trace", skip(self), err)]
    fn get_block(&self, number: BlockNumber) -> GetBlockResponse {
        self.storage
            .get_block(number)
            .ok_or(GetBlockError::NotSynced)
    }
}
