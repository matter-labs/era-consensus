use crate::ctx;
use tokio::time::{timeout, Duration};

// Test scenario:
// 1. Pre-send two sets of 1000 values, so that the first set is expected to be pruned.
// 2. Send a third set of 1000 values in parallel to receiving.
#[tokio::test]
async fn test_prunable_mpsc() {
    let ctx = ctx::test_root(&ctx::RealClock);

    #[derive(Debug, Clone)]
    struct ValueType(usize, usize);

    #[allow(clippy::type_complexity)]
    let (send, mut recv): (super::Sender<ValueType>, super::Receiver<ValueType>) =
        super::channel(|a: &ValueType, b: &ValueType| {
            // Prune values with the same i.
            a.1 == b.1
        });

    let res: Result<(), ctx::Canceled> = crate::scope::run!(&ctx, |ctx, s| async move {
        // Pre-send sets 0 and 1, 1000 values each.
        // Set 0 is expected to be pruned and dropped.
        let values = (0..2000).map(|i| ValueType(i / 1000, i % 1000));
        for val in values {
            send.send(val.clone()).await;
        }
        // Send set 2.
        s.spawn(async move {
            let values = (1000..2000).map(|i| ValueType(2, i));
            for val in values {
                send.send(val.clone()).await;
            }
            Ok(())
        });
        // Receive.
        s.spawn(async move {
            let mut i = 0;
            loop {
                let val = recv.recv(ctx).await.unwrap();
                assert_eq!(val.1, i);
                // Assert the expected set.
                match val.1 {
                    0..=999 => assert_eq!(val.0, 1),
                    1000..=1999 => assert_eq!(val.0, 2),
                    _ => unreachable!(),
                };
                i += 1;
                if i == 2000 {
                    assert!(
                        timeout(Duration::from_secs(0), recv.recv(ctx))
                            .await
                            .is_err(),
                        "recv() is expected to hang since all values have been exhausted"
                    );
                    break;
                }
            }
            Ok(())
        });
        Ok(())
    })
    .await;
    assert_eq!(Ok(()), res);
}
