use tokio::task::yield_now;

use super::*;
use crate::scope;

#[tokio::test]
async fn exclusive_lock_basic_workflow() {
    let ctx = &ctx::test_root(&ctx::RealClock);

    let (mut lock, lock_receiver) = ExclusiveLock::new(1);
    *lock += 4;
    drop(lock);
    let value = lock_receiver.wait(ctx).await.unwrap();
    assert_eq!(value, 5);
}

#[tokio::test]
async fn exclusive_lock_moved_to_task() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    scope::run!(ctx, |ctx, s| async move {
        let (mut lock, lock_receiver) = ExclusiveLock::new("Hello".to_owned());
        s.spawn(async move {
            lock.push_str(", world");
            yield_now().await;
            lock.push('!');
            ctx::OrCanceled::Ok(())
        });

        let value = lock_receiver.wait(ctx).await.unwrap();
        assert_eq!(value, "Hello, world!");
        ctx::OrCanceled::Ok(())
    })
    .await
    .unwrap();
}
