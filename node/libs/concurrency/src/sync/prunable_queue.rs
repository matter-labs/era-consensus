//! Prunable queue mimics a producer-consumer channel, in enabling
//! waiting on the receiver side for new items to arrive, while providing pruning functionality.
//! The pruning takes place whenever a new item is sent, based on a specified predicate.
//!
//! The separation of `Sender` and `Receiver` is employed primarily because `Receiver` requires
//! a mutable reference to the signaling channel, unlike `Sender`, hence making it undesirable to
//! be used in conjunction.

use std::collections::VecDeque;
use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::time::{Duration, timeout};

use crate::{ctx::{self, Ctx}, sync::{self, watch}};

pub fn new<T>(predicate: Box<dyn Sync + Send + Fn(&T, &T) -> bool>) -> (Sender<T>, Receiver<T>) {
    let queue = Mutex::new(VecDeque::new());
    let (tx, rx) = watch::channel(false);

    let shared = Arc::new(Shared {
        queue,
        tx,
    });

    let sender = Sender {
        shared: shared.clone(),
        predicate,
    };

    let receiver = Receiver {
        shared: shared.clone(),
        rx,
    };

    return (sender, receiver);
}

pub struct Shared<T> {
    queue: Mutex<VecDeque<T>>,
    tx: watch::Sender<bool>,
}

pub struct Sender<T> {
    shared: Arc<Shared<T>>,
    predicate: Box<dyn Sync + Send + Fn(&T, &T) -> bool>,
}

impl<T> Sender<T> {
    pub async fn enqueue(&self, item: T) {
        let mut queue = self.shared.queue.lock().await;
        queue.retain(|existing_item| (self.predicate)(existing_item, &item));
        queue.push_back(item);
        self.shared.tx.send(true).unwrap();
    }
}

pub struct Receiver<T> {
    shared: Arc<Shared<T>>,
    rx: watch::Receiver<bool>,
}

impl<T> Receiver<T> {
    pub async fn dequeue(&mut self, ctx: &Ctx) -> ctx::OrCanceled<T> {
        sync::wait_for(ctx, &mut self.rx, |has_items| *has_items).await?;

        let mut queue = self.shared.queue.lock().await;
        let item = queue.pop_front().unwrap();

        if queue.len() == 0 {
            self.shared.tx.send(false).unwrap();
        }

        Ok(item)
    }
}


// Test scenario:
// Enqueue two sets of 0..1000 items, in conjunction, while pruning
// so that only one 0..1000 set is expected to remain in queue.
// Then, dequeue to assert the queue's content.
#[tokio::test]
async fn test_prunable_queue() {
    #[derive(Debug, Clone)]
    struct ItemType(usize, usize);
    let ctx = ctx::test_root(&ctx::RealClock);

    let (sender, mut receiver) = new(
        Box::new(|a: &ItemType, b: &ItemType| {
            a.0 != b.0
        }),
    );

    let sender1 = Arc::new(sender);
    let sender2 = sender1.clone();

    let handle1 = tokio::spawn(async move {
        let set = 1;
        let items = (0..1000).map(|i| ItemType(i, set));
        for item in items {
            sender1.enqueue(item).await;
            tokio::task::yield_now().await;
        }
    });
    let handle2 = tokio::spawn(async move {
        let set = 2;
        let items = (0..1000).map(|i| ItemType(i, set));
        for item in items {
            sender2.enqueue(item).await;
            tokio::task::yield_now().await;
        }
    });
    tokio::try_join!(handle1, handle2);

    tokio::spawn(async move {
        let mut i = 0;
        loop {
            let item = receiver.dequeue(&ctx).await.unwrap();
            assert_eq!(item.0, i);
            i = i + 1;
            if i == 1000 {
                match timeout(Duration::from_secs(0),  receiver.dequeue(&ctx)).await {
                    Ok(_) => assert!(false, "dequeue() is expected to hang since all items have been exhausted"),
                    Err(_) => break
                }
            }
        }
    }).await.unwrap();
}
