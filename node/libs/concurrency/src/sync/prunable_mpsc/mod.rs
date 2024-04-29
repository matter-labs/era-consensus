//! Prunable, multi-producer, single-consumer, unbounded FIFO queue for communicating between asynchronous tasks.
//! The pruning takes place whenever a new value is sent, based on a specified predicate.
//!
//! The separation of [`Sender`] and [`Receiver`] is employed primarily because [`Receiver`] requires
//! a mutable reference to the signaling channel, unlike [`Sender`], hence making it undesirable to
//! be used in conjunction.
//!
use crate::{
    ctx,
    sync::{self, watch},
};
use std::{collections::VecDeque, fmt, sync::Arc};

#[cfg(test)]
mod tests;

/// Creates a channel and returns the [`Sender`] and [`Receiver`] handles.
/// All values sent on [`Sender`] will become available on [`Receiver`] in the same order as it was sent,
/// unless will be pruned before received.
///
/// * [`T`]: The type of data that will be sent through the channel.
/// * [`pruning_predicate`]: A function that determines whether an unreceived, pending value in the buffer (represented by the first `T`) should be pruned
/// based on a newly sent value (represented by the second `T`).
pub fn channel<T>(
    pruning_predicate: impl 'static + Sync + Send + Fn(&T, &T) -> bool,
) -> (Sender<T>, Receiver<T>) {
    let buf = VecDeque::new();
    let (send, recv) = watch::channel(buf);

    let shared = Arc::new(Shared { send });

    let send = Sender {
        shared: shared.clone(),
        pruning_predicate: Box::new(pruning_predicate),
    };

    let recv = Receiver {
        shared: shared.clone(),
        recv,
    };

    (send, recv)
}

struct Shared<T> {
    send: watch::Sender<VecDeque<T>>,
}

/// Sends values to the associated [`Receiver`].
/// Instances are created by the [`channel`] function.
#[allow(clippy::type_complexity)]
pub struct Sender<T> {
    shared: Arc<Shared<T>>,
    pruning_predicate: Box<dyn Sync + Send + Fn(&T, &T) -> bool>,
}

impl<T> Sender<T> {
    /// Sends a value.
    /// This initiates the pruning procedure which operates in O(N) time complexity
    /// on the buffer of pending values.
    pub fn send(&self, value: T) {
        self.shared.send.send_modify(|buf| {
            buf.retain(|pending_value| !(self.pruning_predicate)(pending_value, &value));
            buf.push_back(value);
        });
    }
}

impl<T> fmt::Debug for Sender<T> {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("Sender").finish()
    }
}

/// Receives values from the associated [`Sender`].
/// Instances are created by the [`channel`] function.
pub struct Receiver<T> {
    shared: Arc<Shared<T>>,
    recv: watch::Receiver<VecDeque<T>>,
}

impl<T> Receiver<T> {
    /// Receives the next value for this receiver.
    /// If there are no messages in the buffer, this method will hang until a message is sent.
    pub async fn recv(&mut self, ctx: &ctx::Ctx) -> ctx::OrCanceled<T> {
        sync::wait_for(ctx, &mut self.recv, |buf| !buf.is_empty()).await?;

        let mut value: Option<T> = None;
        self.shared.send.send_modify(|buf| value = buf.pop_front());

        // `None` is unexpected because we waited for new values, and there's only a single receiver.
        Ok(value.unwrap())
    }
}

impl<T> fmt::Debug for Receiver<T> {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("Receiver").finish()
    }
}
