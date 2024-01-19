//! Prunable, multi-producer, single-consumer, unbounded FIFO queue for communicating between asynchronous tasks.
//! The pruning takes place whenever a new value is sent, based on a specified predicate.
//! The channel also facilitates the asynchronous returning of result associated with the processing of values received from the channel.
//!
//! The separation of `Sender` and `Receiver` is employed primarily because `Receiver` requires
//! a mutable reference to the signaling channel, unlike `Sender`, hence making it undesirable to
//! be used in conjunction.
//!
use crate::{
    ctx,
    sync::{Mutex},
    oneshot,
    sync::{self, watch},
};
use std::{collections::VecDeque, fmt, sync::Arc};


/// Creates a channel and returns the `Sender` and `Receiver` handles.
/// All values sent on `Sender` will become available on `Receiver` in the same order as it was sent,
/// unless will be pruned before received.
/// The Sender can be cloned to send to the same channel from multiple code locations. Only one Receiver is supported.
///
/// * `T`: The type of data that will be sent through the channel.
/// * `U`: The type of the asynchronous returning of result associated with the processing of values received from the channel.
/// * `pruning_predicate`: A function that determines whether an unreceived, pending value in the buffer should be pruned based on a newly sent value.
///
pub fn channel<T, U>(
    pruning_predicate: impl 'static + Sync + Send + Fn(&T, &T) -> bool,
) -> (Sender<T, U>, Receiver<T, U>) {
    let queue: Mutex<VecDeque<(T, oneshot::Sender<U>)>> = Mutex::new(VecDeque::new());
    // Internal signaling, to enable waiting on the receiver side for new values.
    let (send, recv) = watch::channel(false);

    let shared = Arc::new(Shared {
        buffer: queue,
        has_values_send: send,
    });

    let sender = Sender {
        shared: shared.clone(),
        pruning_predicate: Box::new(pruning_predicate),
    };

    let receiver = Receiver {
        shared: shared.clone(),
        has_values_recv: recv,
    };

    return (sender, receiver);
}

pub struct Shared<T, U> {
    buffer: Mutex<VecDeque<(T, oneshot::Sender<U>)>>,
    has_values_send: watch::Sender<bool>,
}

pub struct Sender<T, U> {
    shared: Arc<Shared<T, U>>,
    pruning_predicate: Box<dyn Sync + Send + Fn(&T, &T) -> bool>,
}

impl<T, U> Sender<T, U> {
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

pub struct Receiver<T, U> {
    shared: Arc<Shared<T, U>>,
    has_values_recv: watch::Receiver<bool>,
}

impl<T, U> Receiver<T, U> {
    pub async fn recv(&mut self, ctx: &ctx::Ctx) -> ctx::OrCanceled<(T, oneshot::Sender<U>)> {
        sync::wait_for(ctx, &mut self.has_values_recv, |has_values| *has_values).await?;
        let mut buffer = self.shared.buffer.lock().await;
        // `None` is unexpected because we waited for new values.
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

// Test scenario:
// Send two sets of 0..1000 values, in conjunction, while pruning
// so that only one 0..1000 set is expected to remain in the buffer.
// Then, recv to assert the buffer's content.
#[tokio::test]
async fn test_prunable_mpsc() {
    use tokio::time::{timeout, Duration};

    #[derive(Debug, Clone)]
    struct ValueType(usize, usize);

    let ctx = ctx::test_root(&ctx::RealClock);

    let (sender, mut receiver) = channel(|a: &ValueType, b: &ValueType| a.0 != b.0);

    let sender1 = Arc::new(sender);
    let sender2 = sender1.clone();

    let handle1 = tokio::spawn(async move {
        let set = 1;
        let values = (0..1000).map(|i| ValueType(i, set));
        for value in values {
            let _ = sender1.send(value).await;
            tokio::task::yield_now().await;
        }
    });
    let handle2 = tokio::spawn(async move {
        let set = 2;
        let values = (0..1000).map(|i| ValueType(i, set));
        for value in values {
            let _ = sender2.send(value).await;
            tokio::task::yield_now().await;
        }
    });
    tokio::try_join!(handle1, handle2).unwrap();

    tokio::spawn(async move {
        let mut i = 0;
        loop {
            let (value, sender) = receiver.recv(&ctx).await.unwrap();
            assert_eq!(value.0, i);
            let _ = sender.send(());

            i = i + 1;
            if i == 1000 {
                match timeout(Duration::from_secs(0), receiver.recv(&ctx)).await {
                    Ok(_) => assert!(
                        false,
                        "recv() is expected to hang since all values have been exhausted"
                    ),
                    Err(_) => break,
                }
            }
        }
    })
    .await
    .unwrap();
}
