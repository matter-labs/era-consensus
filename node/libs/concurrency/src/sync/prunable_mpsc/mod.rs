//! Prunable, multi-producer, single-consumer, unbounded FIFO queue for communicating between asynchronous tasks.
//! The pruning takes place whenever a new value is sent, based on a specified predicate.
//! The channel also facilitates the asynchronous returning of result associated with the processing of values received from the channel.
//!
//! The separation of [`Sender`] and [`Receiver`] is employed primarily because [`Receiver`] requires
//! a mutable reference to the signaling channel, unlike [`Sender`], hence making it undesirable to
//! be used in conjunction.
//!
use crate::{
    ctx, oneshot,
    sync::{self, watch, Mutex},
};
use std::{collections::VecDeque, fmt, sync::Arc};

#[cfg(test)]
mod tests;

/// Creates a channel and returns the [`Sender`] and [`Receiver`] handles.
/// All values sent on [`Sender`] will become available on [`Receiver`] in the same order as it was sent,
/// unless will be pruned before received.
/// The Sender can be cloned to send to the same channel from multiple code locations. Only one Receiver is supported.
///
/// * [`T`]: The type of data that will be sent through the channel.
/// * [`U`]: The type of the asynchronous returning of result associated with the processing of values received from the channel.
/// * [`pruning_predicate`]: A function that determines whether an unreceived, pending value in the buffer should be pruned based on a newly sent value.
///
pub fn channel<T, U>(
    pruning_predicate: impl 'static + Sync + Send + Fn(&T, &T) -> bool,
) -> (Sender<T, U>, Receiver<T, U>) {
    let queue: Mutex<VecDeque<(T, oneshot::Sender<U>)>> = Mutex::new(VecDeque::new());
    // Internal watch, to enable waiting on the receiver side for new values.
    let (has_values_send, has_values_recv) = watch::channel(false);

    let shared = Arc::new(Shared {
        buffer: queue,
        has_values_send,
    });

    let send = Sender {
        shared: shared.clone(),
        pruning_predicate: Box::new(pruning_predicate),
    };

    let recv = Receiver {
        shared: shared.clone(),
        has_values_recv,
    };

    (send, recv)
}

struct Shared<T, U> {
    buffer: Mutex<VecDeque<(T, oneshot::Sender<U>)>>,
    has_values_send: watch::Sender<bool>,
}

/// Sends values to the associated [`Receiver`].
/// Instances are created by the [`channel`] function.
#[allow(clippy::type_complexity)]
pub struct Sender<T, U> {
    shared: Arc<Shared<T, U>>,
    pruning_predicate: Box<dyn Sync + Send + Fn(&T, &T) -> bool>,
}

impl<T, U> Sender<T, U> {
    /// Sends a value.
    /// This initiates the pruning procedure which operates in O(N) time complexity
    /// on the buffer of pending values.
    pub async fn send(&self, value: T) -> oneshot::Receiver<U> {
        // Create oneshot channel for returning result asynchronously.
        let (res_send, res_recv) = oneshot::channel();

        let mut buffer = self.shared.buffer.lock().await;
        buffer.retain(|pending_value| !(self.pruning_predicate)(&pending_value.0, &value));
        buffer.push_back((value, res_send));

        self.shared.has_values_send.send_replace(true);

        res_recv
    }
}

impl<T, U> fmt::Debug for Sender<T, U> {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("Sender").finish()
    }
}

/// Receives values from the associated [`Sender`].
/// Instances are created by the [`channel`] function.
pub struct Receiver<T, U> {
    shared: Arc<Shared<T, U>>,
    has_values_recv: watch::Receiver<bool>,
}

impl<T, U> Receiver<T, U> {
    /// Receives the next value for this receiver.
    /// If there are no messages in the buffer, this method will hang until a message is sent.
    pub async fn recv(&mut self, ctx: &ctx::Ctx) -> ctx::OrCanceled<(T, oneshot::Sender<U>)> {
        sync::wait_for(ctx, &mut self.has_values_recv, |has_values| *has_values).await?;
        let mut buffer = self.shared.buffer.lock().await;
        // `None` is unexpected because we waited for new values, and there's only a single receiver.
        let value = buffer.pop_front().unwrap();

        if buffer.len() == 0 {
            self.shared.has_values_send.send_replace(false);
        }

        Ok(value)
    }
}

impl<T, U> fmt::Debug for Receiver<T, U> {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("Receiver").finish()
    }
}
