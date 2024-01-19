//! Prunable queue mimics a producer-consumer channel, in enabling
//! waiting on the receiver side for new items, while providing pruning functionality.
//! The pruning takes place whenever a new item is sent, based on a specified predicate.
//!
//! The queue facilitates the asynchronous return of processing result for each added item.
//!
//! The separation of `Sender` and `Receiver` is employed primarily because `Receiver` requires
//! a mutable reference to the signaling channel, unlike `Sender`, hence making it undesirable to
//! be used in conjunction.

use crate::{
    ctx,
    sync::{Mutex},
    oneshot,
    sync::{self, watch},
};
use std::{collections::VecDeque, fmt, sync::Arc};

pub fn channel<T, U>(
    pruning_predicate: impl 'static + Sync + Send + Fn(&T, &T) -> bool,
) -> (Sender<T, U>, Receiver<T, U>) {
    let queue: Mutex<VecDeque<(T, oneshot::Sender<U>)>> = Mutex::new(VecDeque::new());
    // Internal signaling, to enable waiting on the receiver side for new items.
    let (send, recv) = watch::channel(false);

    let shared = Arc::new(Shared {
        queue,
        has_items_send: send,
    });

    let sender = Sender {
        shared: shared.clone(),
        pruning_predicate: Box::new(pruning_predicate),
    };

    let receiver = Receiver {
        shared: shared.clone(),
        has_items_recv: recv,
    };

    return (sender, receiver);
}

pub struct Shared<T, U> {
    queue: Mutex<VecDeque<(T, oneshot::Sender<U>)>>,
    has_items_send: watch::Sender<bool>,
}

pub struct Sender<T, U> {
    shared: Arc<Shared<T, U>>,
    pruning_predicate: Box<dyn Sync + Send + Fn(&T, &T) -> bool>,
}

impl<T, U> Sender<T, U> {
    pub async fn send(&self, item: T) -> oneshot::Receiver<U> {
        // Create oneshot channel for returning result asynchronously.
        let (res_send, res_recv) = oneshot::channel();

        let mut queue = self.shared.queue.lock().await;
        queue.retain(|existing_item| !(self.pruning_predicate)(&existing_item.0, &item));
        queue.push_back((item, res_send));

        self.shared.has_items_send.send_replace(true);

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
    has_items_recv: watch::Receiver<bool>,
}

impl<T, U> Receiver<T, U> {
    pub async fn recv(&mut self, ctx: &ctx::Ctx) -> ctx::OrCanceled<(T, oneshot::Sender<U>)> {
        sync::wait_for(ctx, &mut self.has_items_recv, |has_items| *has_items).await?;
        let mut queue = self.shared.queue.lock().await;
        // `None` is unexpected because we waited for new items.
        let item = queue.pop_front().unwrap();

        if queue.len() == 0 {
            self.shared.has_items_send.send_replace(false);
        }

        Ok(item)
    }
}

impl<T, U> fmt::Debug for Receiver<T, U> {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("Receiver").finish()
    }
}

// Test scenario:
// Send two sets of 0..1000 items, in conjunction, while pruning
// so that only one 0..1000 set is expected to remain in queue.
// Then, recv to assert the queue's content.
#[tokio::test]
async fn test_prunable_queue() {
    use tokio::time::{timeout, Duration};

    #[derive(Debug, Clone)]
    struct ItemType(usize, usize);

    let ctx = ctx::test_root(&ctx::RealClock);

    let (sender, mut receiver) = channel(|a: &ItemType, b: &ItemType| a.0 != b.0);

    let sender1 = Arc::new(sender);
    let sender2 = sender1.clone();

    let handle1 = tokio::spawn(async move {
        let set = 1;
        let items = (0..1000).map(|i| ItemType(i, set));
        for item in items {
            let _ = sender1.send(item).await;
            tokio::task::yield_now().await;
        }
    });
    let handle2 = tokio::spawn(async move {
        let set = 2;
        let items = (0..1000).map(|i| ItemType(i, set));
        for item in items {
            let _ = sender2.send(item).await;
            tokio::task::yield_now().await;
        }
    });
    tokio::try_join!(handle1, handle2).unwrap();

    tokio::spawn(async move {
        let mut i = 0;
        loop {
            let (item, sender) = receiver.recv(&ctx).await.unwrap();
            assert_eq!(item.0, i);
            let _ = sender.send(());

            i = i + 1;
            if i == 1000 {
                match timeout(Duration::from_secs(0), receiver.recv(&ctx)).await {
                    Ok(_) => assert!(
                        false,
                        "recv() is expected to hang since all items have been exhausted"
                    ),
                    Err(_) => break,
                }
            }
        }
    })
    .await
    .unwrap();
}
