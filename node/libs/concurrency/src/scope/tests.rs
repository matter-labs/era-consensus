use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use crate::{ctx, scope, testonly};

// run a trivial future until completion => OK
#[tokio::test]
async fn must_complete_ok() {
    scope::must_complete::Guard.defuse();
}

#[tokio::test]
async fn test_wait_blocking_ok() {
    assert_eq!(5, scope::wait_blocking(|| 5).await);
}

#[tokio::test]
#[should_panic]
async fn test_wait_blocking_panic() {
    scope::wait_blocking(|| panic!("inner panic")).await;
}

type R = Result<(), usize>;

#[test]
fn test_spawn_after_cancelling_scope() {
    testonly::abort_on_panic();
    testonly::with_runtimes(|| async {
        let ctx = &ctx::test_root(&ctx::RealClock);
        let res = scope::run!(ctx, |ctx, s| async {
            s.spawn_blocking(|| R::Err(7));
            ctx.canceled().await;
            s.spawn_blocking(|| R::Err(3));
            Ok(())
        })
        .await;
        assert_eq!(Err(7), res);
    });
}

#[test]
fn test_nested_scopes() {
    testonly::abort_on_panic();
    testonly::with_runtimes(|| async {
        let ctx = &ctx::test_root(&ctx::RealClock);
        let res = scope::run!(ctx, |ctx, s| async {
            s.spawn_blocking(|| {
                scope::run_blocking!(ctx, move |ctx, s| {
                    s.spawn_blocking(|| scope::run_blocking!(ctx, |_, _| { R::Err(8) }));
                    Ok(())
                })
            });
            Ok(())
        })
        .await;
        assert_eq!(Err(8), res);
    });
}

#[test]
fn test_already_canceled() {
    testonly::abort_on_panic();
    testonly::with_runtimes(|| async {
        let ctx = &ctx::test_root(&ctx::RealClock);
        let res = scope::run!(ctx, |ctx, s| async {
            s.cancel();
            // scope::run! should start a task,
            // even though the task has been already canceled.
            scope::run!(ctx, |ctx, s| async {
                s.spawn_blocking(|| {
                    ctx.canceled().block();
                    R::Err(4)
                });
                Ok(())
            })
            .await
        })
        .await;
        assert_eq!(Err(4), res);
    });
}

// After all main tasks complete successfully, the scope gets canceled.
// Background tasks of the scope should still be able to spawn more tasks
// both via `Scope::spawn()` and `Scope::spawn_bg` (although after scope
// cancelation they behave exactly the same).
#[test]
fn test_spawn_from_spawn_bg() {
    testonly::abort_on_panic();
    testonly::with_runtimes(|| async {
        let ctx = &ctx::test_root(&ctx::RealClock);
        let res = scope::run!(ctx, |ctx, s| async {
            s.spawn_bg_blocking(|| {
                ctx.canceled().block();
                s.spawn_blocking(|| {
                    assert!(!ctx.is_active());
                    R::Err(3)
                });
                Ok(())
            });
            Ok(())
        })
        .await;
        assert_eq!(Err(3), res);
    });
}

#[test]
fn test_join() {
    type R = Result<usize, usize>;
    testonly::abort_on_panic();
    testonly::with_runtimes(|| async {
        let ctx = &ctx::test_root(&ctx::RealClock);
        let res = scope::run!(ctx, |ctx, s| async {
            assert_eq!(Ok(5), s.spawn(async { Ok(5) }).join(ctx).await);
            assert_eq!(Ok(9), s.spawn_blocking(|| { Ok(9) }).join(ctx).await);
            assert_eq!(
                Err(ctx::Canceled),
                s.spawn(async { R::Err(9) }).join(ctx).await
            );
            assert!(!ctx.is_active());
            Ok(1)
        })
        .await;
        assert_eq!(Err(9), res);
    });
}

#[test]
fn test_access_to_vars_outside_of_scope() {
    testonly::abort_on_panic();
    testonly::with_runtimes(|| async {
        // Lifetime of `a` is larger than scope's lifetime,
        // so it should be accessible from scope's tasks.
        let a = &AtomicU64::new(0);
        let ctx = &ctx::test_root(&ctx::RealClock);
        scope::run!(ctx, |ctx, s| async {
            s.spawn_blocking(|| {
                scope::run_blocking!(ctx, |_ctx, s| {
                    s.spawn_blocking(|| {
                        a.fetch_add(1, Ordering::Relaxed);
                        Ok(())
                    });
                    Ok(())
                })
            });

            s.spawn_blocking(|| {
                s.spawn_blocking(|| {
                    a.fetch_add(1, Ordering::Relaxed);
                    Ok(())
                });
                a.fetch_add(1, Ordering::Relaxed);
                Ok(())
            });
            a.fetch_add(1, Ordering::Relaxed);
            s.spawn_blocking(|| {
                a.fetch_add(1, Ordering::Relaxed);
                Ok(())
            });
            a.fetch_add(1, Ordering::Relaxed);
            R::Ok(())
        })
        .await
        .unwrap();
        assert_eq!(6, a.load(Ordering::Relaxed));
    });
}

#[tokio::test]
async fn test_wait_for_completion_after_panic_blocking() {
    const N: u64 = 10;
    let a = Arc::new(AtomicU64::new(0));
    let a2 = a.clone();
    let res = tokio::task::spawn_blocking(|| {
        let ctx = &ctx::test_root(&ctx::RealClock);
        let a = a2;
        scope::run_blocking!(ctx, |ctx, s| {
            for _ in 0..N {
                s.spawn(async {
                    ctx.canceled().await;
                    a.fetch_add(1, Ordering::Relaxed);
                    Ok(())
                });
            }
            s.spawn::<()>(async {
                panic!("yolo");
            });
            for _ in 0..N {
                s.spawn_blocking(|| {
                    ctx.canceled().block();
                    a.fetch_add(1, Ordering::Relaxed);
                    Ok(())
                });
            }
            anyhow::Ok(())
        })
    })
    .await;
    assert!(res.is_err());
    assert_eq!(2 * N, a.load(Ordering::Relaxed));
}

#[tokio::test]
async fn test_wait_for_completion_after_panic() {
    const N: u64 = 10;
    let a = Arc::new(AtomicU64::new(0));
    let a2 = a.clone();
    let res = tokio::task::spawn(async {
        let ctx = &ctx::test_root(&ctx::RealClock);
        let a = a2;
        scope::run!(ctx, |ctx, s| async {
            for _ in 0..N {
                s.spawn(async {
                    ctx.canceled().await;
                    a.fetch_add(1, Ordering::Relaxed);
                    Ok(())
                });
            }
            s.spawn::<()>(async {
                panic!("yolo");
            });
            for _ in 0..N {
                s.spawn_blocking(|| {
                    ctx.canceled().block();
                    a.fetch_add(1, Ordering::Relaxed);
                    Ok(())
                });
            }
            anyhow::Ok(())
        })
        .await
    })
    .await;
    assert!(res.is_err());
    assert_eq!(2 * N, a.load(Ordering::Relaxed));
}

#[tokio::test]
async fn test_panic_overrides_error() {
    let res = tokio::task::spawn(async {
        let ctx = &ctx::test_root(&ctx::RealClock);
        let res: anyhow::Result<()> = scope::run!(ctx, |ctx, s| async {
            s.spawn::<()>(async {
                anyhow::bail!("simple error");
            });
            ctx.canceled().await;
            panic!("overriding the error")
        })
        .await;
        res
    })
    .await;
    assert!(res.is_err());
}
