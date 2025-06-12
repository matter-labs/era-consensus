//! Pool of transactions to be gossiped.

use std::collections::{HashSet, VecDeque};

use zksync_concurrency::sync;
use zksync_consensus_engine::{Transaction, TxHash};

/// Pool of transactions to be gossiped.
pub(crate) struct TxPool {
    /// Transactions to be gossiped.
    to_gossip: sync::broadcast::Receiver<Transaction>,
    /// Set of transaction hashes that were already gossiped. We don't want to
    /// gossip the same transactions multiple times. We use a set to quickly
    /// check if a transaction was already gossiped.
    gossiped_set: HashSet<TxHash>,
    /// Queue of transactions hashes that were already gossiped. We keep this queue
    /// to discard old transaction hashes when we run out of memory.
    gossiped_queue: VecDeque<TxHash>,
}

impl TxPool {
    pub(crate) fn new(receiver: sync::broadcast::Receiver<Transaction>) -> Self {
        Self {
            to_gossip: receiver,
            gossiped_set: HashSet::new(),
            gossiped_queue: VecDeque::new(),
        }
    }
}
