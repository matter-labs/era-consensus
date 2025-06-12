//! Pool of transactions to be gossiped.

use std::collections::{HashSet, VecDeque};

use zksync_concurrency::ctx::channel;
use zksync_consensus_engine::{Transaction, TxHash};

/// Sender to the tx pool. It can be used to send transactions to be gossiped.
pub(crate) type TxPoolSender = channel::UnboundedSender<Transaction>;

/// Pool of transactions to be gossiped.
pub(crate) struct TxPool {
    /// Transactions to be gossiped.
    to_gossip: channel::UnboundedReceiver<Transaction>,
    /// Set of transaction hashes that were already gossiped. We don't want to
    /// gossip the same transactions multiple times. We use a set to quickly
    /// check if a transaction was already gossiped.
    gossiped_set: HashSet<TxHash>,
    /// Queue of transactions hashes that were already gossiped. We keep this queue
    /// to discard old transaction hashes when we run out of memory.
    gossiped_queue: VecDeque<TxHash>,
}

impl TxPool {
    pub(crate) fn new() -> (Self, TxPoolSender) {
        let (sender, receiver) = channel::unbounded();
        (
            Self {
                to_gossip: receiver,
                gossiped_set: HashSet::new(),
                gossiped_queue: VecDeque::new(),
            },
            sender,
        )
    }
}
