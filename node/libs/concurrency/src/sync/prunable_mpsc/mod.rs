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

/// Possible results for the filter function
#[derive(Debug, PartialEq)]
pub enum SelectionFunctionResult {
    /// Keep both values.
    Keep,
    /// Discard old value.
    DiscardOld,
    /// Discard new value.
    DiscardNew,
}

/// Creates a channel and returns the [`Sender`] and [`Receiver`] handles.
/// All values sent on [`Sender`] will become available on [`Receiver`] in the same order as it was sent,
/// unless will be pruned before received.
/// The Sender can be cloned to send to the same channel from multiple code locations. Only one Receiver is supported.
///
/// * [`T`]: The type of data that will be sent through the channel.
/// * [`filter_function`]: A function that checks the newly sent value and avoids adding to the queue if it returns false
/// * [`selection_function`]: A function that determines whether an unreceived, pending value in the buffer (represented by the first `T`) should be pruned
/// based on a newly sent value (represented by the second `T`) or the new value should be filtered out, or both should be kept.
/// Both functions return a SelectionFunctionResult for code clarity, but the filter_function should never return a DiscardOld result
/// as it is not used against any pending message in the queue.
pub fn channel<T>(
    filter_function: impl 'static + Sync + Send + Fn(&T) -> SelectionFunctionResult,
    selection_function: impl 'static + Sync + Send + Fn(&T, &T) -> SelectionFunctionResult,
) -> (Sender<T>, Receiver<T>) {
    let buf = VecDeque::new();
    let (send, recv) = watch::channel(buf);

    let shared = Arc::new(Shared { send });

    let send = Sender {
        shared: shared.clone(),
        filter_function: Box::new(filter_function),
        selection_function: Box::new(selection_function),
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
    filter_function: Box<dyn Sync + Send + Fn(&T) -> SelectionFunctionResult>,
    selection_function: Box<dyn Sync + Send + Fn(&T, &T) -> SelectionFunctionResult>,
}

impl<T> Sender<T> {
    /// Sends a value.
    /// This initiates the filter and pruning procedure which operates in O(N) time complexity
    /// on the buffer of pending values.
    pub fn send(&self, value: T) {
        // Check if new value is valid to keep
        if (self.filter_function)(&value) != SelectionFunctionResult::Keep {
            return;
        }
        self.shared.send.send_modify(|buf| {
            let len = buf.len();
            let mut idx = 0;
            let mut cur = 0;

            // Stage 1: All values are retained.
            while cur < len {
                match (self.selection_function)(&mut buf[cur], &value) {
                    SelectionFunctionResult::Keep => {
                        cur += 1;
                        idx += 1;
                    }
                    SelectionFunctionResult::DiscardOld => {
                        cur += 1;
                        break;
                    }
                    SelectionFunctionResult::DiscardNew => return,
                }
            }
            // Stage 2: Swap retained value into current idx.
            while cur < len {
                match (self.selection_function)(&mut buf[cur], &value) {
                    SelectionFunctionResult::Keep => {
                        buf.swap(idx, cur);
                        cur += 1;
                        idx += 1;
                    }
                    SelectionFunctionResult::DiscardOld => {
                        cur += 1;
                        continue;
                    }
                    SelectionFunctionResult::DiscardNew => {
                        if cur != idx {
                            buf.truncate(idx);
                        }
                        return;
                    }
                }
            }
            // Stage 3: Truncate all values after idx.
            if cur != idx {
                buf.truncate(idx);
            }

            // Finally, push the value to the end of the queue.
            buf.push_back(value)
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
