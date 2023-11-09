//! Module to manage the communication between actors. It simply converts and forwards messages from and to each different actor.

use crate::metrics;
use tracing::instrument;
use zksync_concurrency::{
    ctx::{self, channel},
    scope,
};
use zksync_consensus_bft::io::{
    InputMessage as ConsensusInputMessage, OutputMessage as ConsensusOutputMessage,
};
use zksync_consensus_network::io::{
    InputMessage as NetworkInputMessage, OutputMessage as NetworkOutputMessage,
};
use zksync_consensus_roles::validator::FinalBlock;
use zksync_consensus_sync_blocks::io::{
    InputMessage as SyncBlocksInputMessage, OutputMessage as SyncBlocksOutputMessage,
};
use zksync_consensus_utils::pipe::DispatcherPipe;

/// The IO dispatcher, it is the main struct to handle actor messages. It simply contains a sender and a receiver for
/// a pair of channels for each actor. This of course allows us to send and receive messages to and from each actor.
#[derive(Debug)]
pub(super) struct Dispatcher {
    consensus_input: channel::UnboundedSender<ConsensusInputMessage>,
    consensus_output: channel::UnboundedReceiver<ConsensusOutputMessage>,
    sync_blocks_input: channel::UnboundedSender<SyncBlocksInputMessage>,
    sync_blocks_output: channel::UnboundedReceiver<SyncBlocksOutputMessage>,
    network_input: channel::UnboundedSender<NetworkInputMessage>,
    network_output: channel::UnboundedReceiver<NetworkOutputMessage>,
    blocks_sender: channel::UnboundedSender<FinalBlock>,
}

impl Dispatcher {
    /// Creates a new IO Dispatcher.
    pub(super) fn new(
        consensus_pipe: DispatcherPipe<ConsensusInputMessage, ConsensusOutputMessage>,
        sync_blocks_pipe: DispatcherPipe<SyncBlocksInputMessage, SyncBlocksOutputMessage>,
        network_pipe: DispatcherPipe<NetworkInputMessage, NetworkOutputMessage>,
        blocks_sender: channel::UnboundedSender<FinalBlock>,
    ) -> Self {
        Dispatcher {
            consensus_input: consensus_pipe.send,
            consensus_output: consensus_pipe.recv,
            sync_blocks_input: sync_blocks_pipe.send,
            sync_blocks_output: sync_blocks_pipe.recv,
            network_input: network_pipe.send,
            network_output: network_pipe.recv,
            blocks_sender,
        }
    }

    /// Method to start the IO dispatcher. It is simply a loop to receive messages from the actors and then forward them.
    #[instrument(level = "trace", ret)]
    pub(super) fn run(&mut self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        scope::run_blocking!(ctx, |ctx, s| {
            // Start a task to handle the messages from the consensus actor.
            s.spawn(async {
                while let Ok(msg) = self.consensus_output.recv(ctx).await {
                    match msg {
                        ConsensusOutputMessage::Network(message) => {
                            self.network_input.send(message.into());
                        }
                        ConsensusOutputMessage::FinalizedBlock(block) => {
                            let number_metric = &metrics::METRICS.finalized_block_number;
                            let current_number = number_metric.get();
                            number_metric.set(current_number.max(block.header.number.0));
                            // This works because this is the only place where `finalized_block_number`
                            // is modified, and there should be a single running `Dispatcher`.

                            self.blocks_sender.send(block);
                        }
                    }
                }
                Ok(())
            });

            s.spawn(async {
                while let Ok(msg) = self.sync_blocks_output.recv(ctx).await {
                    match msg {
                        SyncBlocksOutputMessage::Network(message) => {
                            self.network_input.send(message.into());
                        }
                    }
                }
                Ok(())
            });

            // Start a task to handle the messages from the network actor.
            s.spawn(async {
                while let Ok(msg) = self.network_output.recv(ctx).await {
                    match msg {
                        NetworkOutputMessage::Consensus(message) => {
                            self.consensus_input
                                .send(ConsensusInputMessage::Network(message));
                        }
                        NetworkOutputMessage::SyncBlocks(message) => {
                            self.sync_blocks_input
                                .send(SyncBlocksInputMessage::Network(message));
                        }
                    }
                }
                Ok(())
            });

            Ok(())
        })
    }
}
