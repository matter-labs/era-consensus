use assert_matches::assert_matches;

use super::*;
use crate::ctx;

// Test scenario:
// 1. Pre-send two sets of 1000 values, so that the first set is expected to be pruned.
// 2. Send a third set of 1000 values in parallel to receiving.
#[tokio::test]
async fn test_prunable_mpsc() {
    crate::testonly::abort_on_panic();
    let ctx = ctx::test_root(&ctx::RealClock);

    #[derive(Debug, Clone)]
    struct ValueType(usize, usize);

    let (send, recv): (Sender<ValueType>, Receiver<ValueType>) = channel(
        |_: &ValueType| true,
        |a, b| {
            // Prune values with the same i.
            if a.1 == b.1 {
                SelectionFunctionResult::DiscardOld
            } else {
                SelectionFunctionResult::Keep
            }
        },
    );

    let res: Result<(), ctx::Canceled> = crate::scope::run!(&ctx, |ctx, s| async move {
        // Pre-send sets 0 and 1, 1000 values each.
        // Set 0 is expected to be pruned and dropped.
        let values = (0..2000).map(|i| ValueType(i / 1000, i % 1000));
        for val in values {
            send.send(val);
        }
        // Send set 2.
        s.spawn(async {
            let send = send;
            let values = (1000..2000).map(|i| ValueType(2, i));
            for val in values {
                send.send(val.clone());
            }
            Ok(())
        });
        // Receive.
        s.spawn(async {
            let mut recv = recv;
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
                    assert_matches!(
                        recv.recv(&ctx.with_timeout(time::Duration::milliseconds(10)))
                            .await,
                        Err(ctx::Canceled),
                        "recv() is expected to hang and be canceled since all values have been \
                         exhausted"
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

// Non-parallel test scenario:
// Send five sets of 10 values:
//   - Set 1 will be discarded at reception
//   - Sets 2 and 3 will be admitted at reception
//   - Set 4 will cause set 2 to be pruned
//   - Set 5 will be discarded on conflict with values on Set 3
//   - Eventually, only Set 3 and Set 4 will be consumed from the queue
#[tokio::test]
async fn test_prunable_mpsc_sanity() {
    crate::testonly::abort_on_panic();
    let ctx = ctx::test_root(&ctx::RealClock);

    #[derive(Debug, Clone)]
    struct ValueType {
        id: usize,
        ord: usize,
        set: usize,
    }

    let (send, recv): (Sender<ValueType>, Receiver<ValueType>) = channel(
        |new_value: &ValueType| {
            // Discard values from set 1
            new_value.set != 1
        },
        |old_value, new_value| {
            // Discard values with the same id and lower ord.
            if old_value.id == new_value.id {
                if old_value.ord < new_value.ord {
                    SelectionFunctionResult::DiscardOld
                } else {
                    SelectionFunctionResult::DiscardNew
                }
            } else {
                SelectionFunctionResult::Keep
            }
        },
    );

    // Send set 1 with 10 values. Will be discarded before entering the queue
    for val in (1..=10).map(|i| ValueType {
        id: i,
        ord: 1,
        set: 1,
    }) {
        send.send(val);
    }
    // Send set 2. Will be enqueued: ids 1 to 10
    for val in (1..=10).map(|i| ValueType {
        id: i,
        ord: 1,
        set: 2,
    }) {
        send.send(val);
    }
    // Send set 3. Will be enqueued: ids 11 to 20
    for val in (1..=10).map(|i| ValueType {
        id: i + 10,
        ord: 2,
        set: 3,
    }) {
        send.send(val);
    }
    // Send set 4. Will prune set 2 as ids match but ord is larger
    for val in (1..=10).map(|i| ValueType {
        id: i,
        ord: 3,
        set: 4,
    }) {
        send.send(val);
    }
    // Send set 5. Will be discarded as ids match with set 3 but ord is smaller
    for val in (1..=10).map(|i| ValueType {
        id: i + 10,
        ord: 1,
        set: 5,
    }) {
        send.send(val);
    }
    // Receive.
    let mut recv = recv;
    let mut i = 1;
    loop {
        let val = recv.recv(&ctx).await.unwrap();

        // Assert the expected sets.
        match val.id {
            // values with id <= 10 should have been replaced by set 4
            1..=10 => assert_eq!(val.set, 4),
            // values with id <= 20 should be from set 3
            11..=20 => assert_eq!(val.set, 3),
            _ => unreachable!(),
        };
        i += 1;
        if i == 21 {
            assert_matches!(
                recv.recv(&ctx.with_timeout(time::Duration::milliseconds(10)))
                    .await,
                Err(ctx::Canceled),
                "recv() is expected to hang and be canceled since all values have been exhausted"
            );
            break;
        }
    }
}
