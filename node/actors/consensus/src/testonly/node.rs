use super::Fuzz;
use crate::io;
use concurrency::{ctx, ctx::channel, scope};
use network::io::ConsensusInputMessage;
use rand::Rng;
use roles::validator;
use utils::pipe::DispatcherPipe;

/// A struct containing metrics information. Right now it's just a finalized block.
#[derive(Debug)]
pub(super) struct Metrics {
    pub(crate) validator: validator::PublicKey,
    pub(crate) finalized_block: validator::FinalBlock,
}

/// Enum representing the behavior of the node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Behavior {
    /// A replica that is always online and behaves honestly.
    Honest,
    /// A replica that is always offline and does not produce any messages.
    Offline,
    /// A replica that is always online and behaves randomly. It will produce
    /// completely random messages.
    Random,
    /// A replica that is always online and behaves maliciously. It will produce
    /// realistic but wrong messages.
    Byzantine,
}

/// Struct representing a node.
pub(super) struct Node {
    pub(crate) net: network::testonly::Instance,
    pub(crate) behavior: Behavior,
}

impl Node {
    /// Runs a mock executor.
    pub(crate) async fn run_executor(
        &self,
        ctx: &ctx::Ctx,
        consensus_pipe: DispatcherPipe<io::InputMessage, io::OutputMessage>,
        network_pipe: DispatcherPipe<network::io::InputMessage, network::io::OutputMessage>,
        metrics: channel::UnboundedSender<Metrics>,
    ) -> anyhow::Result<()> {
        let key = self.net.consensus_config().key.public();
        let rng = &mut ctx.rng();
        let mut net_recv = network_pipe.recv;
        let net_send = network_pipe.send;
        let mut con_recv = consensus_pipe.recv;
        let con_send = consensus_pipe.send;

        scope::run!(ctx, |ctx, s| async {
            s.spawn(async {
                while let Ok(network_message) = net_recv.recv(ctx).await {
                    match network_message {
                        network::io::OutputMessage::Consensus(req) => {
                            con_send.send(io::InputMessage::Network(req));
                        }
                        network::io::OutputMessage::SyncBlocks(_) => {
                            // Drop message related to block syncing; the nodes should work fine
                            // without them.
                        }
                    }
                }
                Ok(())
            });
            // Get the next message from the channel. Our response depends on what type of replica we are.
            while let Ok(msg) = con_recv.recv(ctx).await {
                match msg {
                    io::OutputMessage::FinalizedBlock(block) => {
                        // Send the finalized block to the watcher.
                        metrics.send(Metrics {
                            validator: key.clone(),
                            finalized_block: block,
                        })
                    }
                    io::OutputMessage::Network(mut message) => {
                        let message_to_send = match self.behavior {
                            Behavior::Offline => continue,
                            Behavior::Honest => message,
                            // Create a random consensus message and broadcast.
                            Behavior::Random => ConsensusInputMessage {
                                message: rng.gen(),
                                recipient: network::io::Target::Broadcast,
                            },
                            Behavior::Byzantine => {
                                message.message.mutate(rng);
                                message
                            }
                        };
                        net_send.send(message_to_send.into());
                    }
                }
            }
            Ok(())
        })
        .await
    }
}
