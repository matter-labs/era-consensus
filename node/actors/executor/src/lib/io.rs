//! Module to manage the communication between actors. It simply converts and forwards messages from and to each different actor.

use concurrency::{
    ctx::{self, channel},
    scope,
};
use consensus::io::{
    InputMessage as ConsensusInputMessage, OutputMessage as ConsensusOutputMessage,
};
use network::io::{InputMessage as NetworkInputMessage, OutputMessage as NetworkOutputMessage};
use once_cell::sync::Lazy;
use sync_blocks::io::{
    InputMessage as SyncBlocksInputMessage, OutputMessage as SyncBlocksOutputMessage,
};
use tracing::instrument;
use utils::pipe::DispatcherPipe;

static FINALIZED_BLOCK_NUMBER: Lazy<prometheus::IntGauge> = Lazy::new(|| {
    prometheus::register_int_gauge!(
        "executor_lib_io__finalized_block_number",
        "Number of the last finalized block observed by the node",
    )
    .unwrap()
});

/// The IO dispatcher, it is the main struct to handle actor messages. It simply contains a sender and a receiver for
/// a pair of channels for each actor. This of course allows us to send and receive messages to and from each actor.
#[derive(Debug)]
pub struct Dispatcher {
    consensus_input: channel::UnboundedSender<ConsensusInputMessage>,
    consensus_output: channel::UnboundedReceiver<ConsensusOutputMessage>,
    sync_blocks_input: channel::UnboundedSender<SyncBlocksInputMessage>,
    network_input: channel::UnboundedSender<NetworkInputMessage>,
    network_output: channel::UnboundedReceiver<NetworkOutputMessage>,
}

impl Dispatcher {
    /// Creates a new IO Dispatcher.
    pub fn new(
        consensus_pipe: DispatcherPipe<ConsensusInputMessage, ConsensusOutputMessage>,
        sync_blocks_pipe: DispatcherPipe<SyncBlocksInputMessage, SyncBlocksOutputMessage>,
        network_pipe: DispatcherPipe<NetworkInputMessage, NetworkOutputMessage>,
    ) -> Self {
        Dispatcher {
            consensus_input: consensus_pipe.send,
            consensus_output: consensus_pipe.recv,
            sync_blocks_input: sync_blocks_pipe.send,
            network_input: network_pipe.send,
            network_output: network_pipe.recv,
        }
    }

    /// Method to start the IO dispatcher. It is simply a loop to receive messages from the actors and then forward them.
    #[instrument(level = "trace", ret)]
    pub fn run(&mut self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        scope::run_blocking!(ctx, |ctx, s| {
            // Start a task to handle the messages from the consensus actor.
            s.spawn(async {
                while let Ok(msg) = self.consensus_output.recv(ctx).await {
                    match msg {
                        ConsensusOutputMessage::Network(message) => {
                            self.network_input.send(message.into())
                        }
                        ConsensusOutputMessage::FinalizedBlock(b) => {
                            let n = FINALIZED_BLOCK_NUMBER.get();
                            FINALIZED_BLOCK_NUMBER.set(n.max(b.block.number.0 as i64));
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
