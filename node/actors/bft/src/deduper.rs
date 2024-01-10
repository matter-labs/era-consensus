use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(crate) struct Deduper<T> {
    queue: Mutex<VecDeque<T>>, // TODO: don't bound to a concrete type
    predicate: Box<dyn Sync + Send + Fn(&T, &T) -> bool>,
}

impl<T> Deduper<T> {
    pub fn new(queue: Mutex<VecDeque<T>>, predicate: Box<dyn Sync + Send + Fn(&T, &T) -> bool>) -> Self {
        Self {
            queue,
            predicate,
        }
    }

    pub fn enqueue(&self, item: T) {
        let mut queue = self.queue.lock().unwrap();
        queue.retain(|existing_item| (self.predicate)(existing_item, &item));
        queue.push_back(item)
    }

    pub async fn dequeue(&self) -> Option<T> {
        // TODO: await/block until next item is available
        let mut queue = self.queue.lock().unwrap();
        queue.pop_front()
    }
}

// TODO: this is not a proper test yet, just a temp debugging tool
#[tokio::test]
async fn test_deduper() {
    #[derive(PartialEq, Debug)]
    struct MockType {
        id: usize,
    }
    let mut deduper = Arc::new(Deduper::new(
        Mutex::new(VecDeque::<MockType>::new()),
        Box::new(|a: &MockType, b: &MockType| {
            a.id != b.id
        }),
    ));

    let mut deduper_clone = deduper.clone();
    let handle1 = tokio::spawn(async move {
        for i in 1..=90 {
            deduper_clone.enqueue(MockType { id: i });
            tokio::task::yield_now().await;
            // println!("{:?}", i)
        }
        tokio::time::sleep(Duration::from_millis(1)).await;

        for i in 100..=190 {
            deduper_clone.enqueue(MockType { id: i })
        }
        tokio::time::sleep(Duration::from_millis(1)).await;

        for i in 200..=290 {
            deduper_clone.enqueue(MockType { id: i })
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    });

    let mut deduper_clone = deduper.clone();
    let handle2 = tokio::spawn(async move {
        let mut counter = 1;
        loop {
            let item = deduper_clone.dequeue().await;
            match item {
                Some(item) => {
                    println!("{:?} {:?}", counter, item);
                }
                None => {
                    println!("{:?} {:?}", counter, item);
                }
            }

            counter += 1;
            tokio::task::yield_now().await
        }
    });

    let _ = tokio::join!(handle1);
}