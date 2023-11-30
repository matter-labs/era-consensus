use super::*;
use crate::{scope, testonly};

#[tokio::test]
async fn test_run_canceled() {
    testonly::abort_on_panic();
    let ctx = &root();
    assert!(ctx.is_active());
    scope::run!(ctx, |ctx, s| async {
        s.cancel();
        assert!(!ctx.is_active());
        Ok::<(), ()>(())
    })
    .await
    .unwrap();
    assert!(ctx.is_active());
}

#[test]
fn test_clock_advance() {
    testonly::abort_on_panic();
    let clock = ManualClock::new();
    let ctx = &test_root(&clock);
    let now = ctx.now();
    let now_utc = ctx.now_utc();
    let delta = time::Duration::seconds(10);
    clock.advance(delta);
    assert_eq!(ctx.now(), now + delta);
    assert_eq!(ctx.now_utc(), now_utc + delta);
}

#[tokio::test]
async fn test_sleep_until() {
    type R<E> = std::result::Result<(), E>;
    testonly::abort_on_panic();
    let clock = ManualClock::new();
    let ctx = &test_root(&clock);
    let sec = time::Duration::SECOND;

    let t = ctx.now() + 1000 * sec;
    let res = scope::run!(ctx, |ctx, s| async {
        s.spawn(async {
            ctx.sleep_until(t).await.unwrap();
            tracing::info!("subtask terminating");
            R::Err(9)
        });
        clock.advance(1001 * sec);
        tracing::info!("root task terminating");
        Ok(())
    })
    .await;
    assert_eq!(Err(9), res);

    let t = ctx.now() + 1000 * sec;
    let res = scope::run!(ctx, |ctx, s| async {
        s.spawn(async {
            assert!(ctx.sleep_until(t).await.is_err());
            Ok(())
        });
        clock.advance_until(t - sec);
        R::Err(1)
    })
    .await;
    assert_eq!(Err(1), res);
}

#[tokio::test]
async fn channel_recv() {
    testonly::abort_on_panic();
    let ctx = &root();
    let (send, mut recv) = channel::unbounded();
    send.send(1);
    assert_eq!(1, recv.recv(ctx).await.unwrap());

    let res = scope::run!(ctx, |ctx, s| async {
        s.cancel();
        recv.recv(ctx).await
    })
    .await;
    assert!(matches!(res, Err(Canceled)));
}

#[tokio::test]
async fn channel_send() {
    testonly::abort_on_panic();
    let (send, recv) = channel::unbounded();
    drop(recv);
    // Send should succeed, even if recv has been dropped.
    send.send(3);
}
