//! Module to manage the communication between actors. It simply converts and forwards messages from and to each different actor.
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
use zksync_consensus_utils::pipe::DispatcherPipe;

/// The IO dispatcher, it is the main struct to handle actor messages. It simply contains a sender and a receiver for
/// a pair of channels for each actor. This of course allows us to send and receive messages to and from each actor.
#[derive(Debug)]
pub(super) struct Dispatcher {
    consensus_input: channel::UnboundedSender<ConsensusInputMessage>,
    consensus_output: channel::UnboundedReceiver<ConsensusOutputMessage>,
    network_input: channel::UnboundedSender<NetworkInputMessage>,
    network_output: channel::UnboundedReceiver<NetworkOutputMessage>,
}

impl Dispatcher {
    /// Creates a new IO Dispatcher.
    pub(super) fn new(
        consensus_pipe: DispatcherPipe<ConsensusInputMessage, ConsensusOutputMessage>,
        network_pipe: DispatcherPipe<NetworkInputMessage, NetworkOutputMessage>,
    ) -> Self {
        Dispatcher {
            consensus_input: consensus_pipe.send,
            consensus_output: consensus_pipe.recv,
            network_input: network_pipe.send,
            network_output: network_pipe.recv,
        }
    }

    /// Method to start the IO dispatcher. It is simply a loop to receive messages from the actors and then forward them.
    #[instrument(level = "trace", ret)]
    pub(super) async fn run(mut self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        scope::run!(ctx, |ctx, s| async {
            // Start a task to handle the messages from the consensus actor.
            s.spawn(async {
                while let Ok(msg) = self.consensus_output.recv(ctx).await {
                    match msg {
                        ConsensusOutputMessage::Network(message) => {
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
                    }
                }
                Ok(())
            });

            Ok(())
        }).await
    }
}
