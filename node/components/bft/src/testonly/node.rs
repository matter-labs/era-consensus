use crate::{testonly, FromNetworkMessage, PayloadManager, ToNetworkMessage};
use anyhow::Context as _;
use std::sync::Arc;
use zksync_concurrency::time;
use zksync_concurrency::{ctx, ctx::channel, scope, sync};
use zksync_consensus_network as network;
use zksync_consensus_storage as storage;
use zksync_consensus_storage::testonly::in_memory;

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
        consensus_receiver: sync::prunable_mpsc::Receiver<FromNetworkMessage>,
        consensus_sender: channel::UnboundedSender<ToNetworkMessage>,
    ) -> anyhow::Result<()> {
        scope::run!(ctx, |ctx, s| async {
            // Create a channel for the consensus component to send messages to the network.
            // We will use this extra channel to filter messages depending on the nodes
            // behavior.
            let (net_send, mut net_recv) = channel::unbounded();

            // Run the consensus component
            s.spawn(async {
                let validator_key = self.net.validator_key.clone().unwrap();
                crate::Config {
                    secret_key: validator_key.clone(),
                    block_store: self.block_store.clone(),
                    replica_store: Box::new(in_memory::ReplicaStore::default()),
                    payload_manager: self.behavior.payload_manager(),
                    max_payload_size: MAX_PAYLOAD_SIZE,
                    view_timeout: time::Duration::milliseconds(2000),
                }
                .run(ctx, net_send, consensus_receiver)
                .await
                .context("consensus.run()")
            });

            // Forward output messages from the consensus to the network;
            // turns output from this to inputs for others.
            // Get the next message from the channel. Our response depends on what type of replica we are.
            while let Ok(msg) = net_recv.recv(ctx).await {
                match self.behavior {
                    Behavior::Offline => (),
                    Behavior::Honest | Behavior::HonestNotProposing => consensus_sender.send(msg),
                };
            }
            Ok(())
        })
        .await
    }
}
