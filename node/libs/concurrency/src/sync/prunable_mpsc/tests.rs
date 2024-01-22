use tokio::time::{Duration, timeout};
use crate::ctx;
use assert_matches::assert_matches;

// Test scenario:
// 1. Pre-send two sets of 1000 values, so that the first set is expected to be pruned.
// 2. Send a third set of 1000 values in parallel to receiving.
#[tokio::test]
async fn test_prunable_mpsc() {
    let ctx = ctx::test_root(&ctx::RealClock);

    #[derive(Debug, Clone)]
    struct ValueType(usize, usize);

    let (send, mut recv): (
        super::Sender<ValueType, Result<(), usize>>,
        super::Receiver<ValueType, Result<(), usize>>,
    ) = super::channel(|a: &ValueType, b: &ValueType| {
        // Prune values with the same i.
        a.1 == b.1
    });

    let res: Result<(), ctx::Canceled> = crate::scope::run!(&ctx, |ctx, s| async move {
        // Pre-send sets 0 and 1, 1000 values each.
        // Set 0 is expected to be pruned and dropped.
        let values = (0..2000).map(|i| {
            ValueType(i/1000, i%1000)
        });
        for val in values {
            let res_recv = send.send(val.clone()).await;
            s.spawn(async move {
                let res = res_recv.recv_or_disconnected(ctx).await;
                match val.0 {
                    // set 0 values are expected to be pruned and dropped.
                    0 => assert_matches!(res, Ok(Err(crate::sync::Disconnected))),
                    // set 1 values are expected to return `Ok(())`.
                    1 => assert_matches!(res, Ok(Ok(Ok(())))),
                    _ => unreachable!()
                }
                Ok(())
            });
        }
        // Send set 2.
        s.spawn(async move {
            let values = (1000..2000).map(|i| ValueType(2, i));
            for val in values {
                let res_recv = send.send(val.clone()).await;
                s.spawn(async move {
                    let res = res_recv.recv_or_disconnected(ctx).await;
                    let i = val.1;
                    match val.0 {
                        // set 2 values are expected to return `Err(i)`.
                        2 => assert_matches!(res, Ok(Ok(Err(err))) => {
                            assert_eq!(err, i);
                        }),
                        _ => unreachable!()
                    };
                    Ok(())
                });
            }
            Ok(())
        });
        // Receive.
        s.spawn(async move {
            let mut i = 0;
            loop {
                let (val, res_send) = recv.recv(ctx).await.unwrap();
                assert_eq!(val.1, i);
                match val.0 {
                    // set 0 is expected to be pruned and dropped.
                    0 => unreachable!(),
                    // Return `Ok(())` for set 1.
                    1 => res_send.send(Ok(())).unwrap(),
                    // Return `Err(i)` for set 2.
                    2 => res_send.send(Err(i)).unwrap(),
                    _ => unreachable!(),
                };
                i = i + 1;
                if i == 2000 {
                    assert!(
                        timeout(Duration::from_secs(0), recv.recv(ctx)).await.is_err(),
                        "recv() is expected to hang since all values have been exhausted"
                    );
                    break;
                }
            }
            Ok(())
        });
        Ok(())
    }).await;
    assert_eq!(Ok(()), res);
}