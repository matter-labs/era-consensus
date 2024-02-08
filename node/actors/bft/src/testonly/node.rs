use super::Fuzz;
use crate::{io, testonly, PayloadManager};
use anyhow::Context as _;
use rand::Rng;
use std::sync::Arc;
use zksync_concurrency::{ctx, scope};
use zksync_consensus_network as network;
use zksync_consensus_network::io::ConsensusInputMessage;
use zksync_consensus_storage as storage;
use zksync_consensus_storage::testonly::in_memory;
use zksync_consensus_utils::pipe;

pub(crate) const MAX_PAYLOAD_SIZE: usize = 1000;

/// Enum representing the behavior of the node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Behavior {
    /// A replica that is always online and behaves honestly.
    Honest,
    /// Same as honest, except that it never proposes a block (which is a legit behavior)
    HonestNotProposing,
    /// A replica that is always offline and does not produce any messages.
    Offline,
    /// A replica that is always online and behaves randomly. It will produce
    /// completely random messages.
    Random,
    /// A replica that is always online and behaves maliciously. It will produce
    /// realistic but wrong messages.
    Byzantine,
}

impl Behavior {
    pub(crate) fn payload_manager(&self) -> Box<dyn PayloadManager> {
        match self {
            Self::HonestNotProposing => Box::new(testonly::PendingPayload),
            _ => Box::new(testonly::RandomPayload(MAX_PAYLOAD_SIZE)),
        }
    }
}

/// Struct representing a node.
pub(super) struct Node {
    pub(crate) net: network::Config,
    pub(crate) behavior: Behavior,
    pub(crate) block_store: Arc<storage::BlockStore>,
}

impl Node {
    /// Runs a mock executor.
    pub(crate) async fn run(
        &self,
        ctx: &ctx::Ctx,
        network_pipe: &mut pipe::DispatcherPipe<
            network::io::InputMessage,
            network::io::OutputMessage,
        >,
    ) -> anyhow::Result<()> {
        let rng = &mut ctx.rng();
        let net_recv = &mut network_pipe.recv;
        let net_send = &mut network_pipe.send;
        let (consensus_actor_pipe, consensus_pipe) = pipe::new();
        let mut con_recv = consensus_pipe.recv;
        let con_send = consensus_pipe.send;
        scope::run!(ctx, |ctx, s| async {
            s.spawn(async {
                let validator_key = self.net.validator_key.clone().unwrap();
                crate::Config {
                    secret_key: validator_key.clone(),
                    validator_set: self.net.genesis.validators.clone(),
                    block_store: self.block_store.clone(),
                    replica_store: Box::new(in_memory::ReplicaStore::default()),
                    payload_manager: self.behavior.payload_manager(),
                    max_payload_size: MAX_PAYLOAD_SIZE,
                }
                .run(ctx, consensus_actor_pipe)
                .await
                .context("consensus.run()")
            });
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
                    io::OutputMessage::Network(mut message) => {
                        let message_to_send = match self.behavior {
                            Behavior::Offline => continue,
                            Behavior::Honest | Behavior::HonestNotProposing => message,
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
