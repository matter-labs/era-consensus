use std::sync::Arc;
use crate::ctx;

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

    let (sender, mut receiver) = super::channel(|a: &ValueType, b: &ValueType| a.0 == b.0);

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
                assert!(
                    timeout(Duration::from_secs(0), receiver.recv(&ctx)).await.is_err(),
                    "recv() is expected to hang since all values have been exhausted"
                );
                break;
            }
        }
    })
        .await
        .unwrap();
}
