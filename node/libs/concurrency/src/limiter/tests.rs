use super::*;
use crate::{ctx, scope, testonly, time};
use rand::Rng;

#[tokio::test]
async fn immediate_permit_consumption() {
    testonly::abort_on_panic();
    let clock = &ctx::ManualClock::new();
    clock.set_advance_on_sleep();
    let ctx = &ctx::test_root(clock);
    let rate = Rate {
        burst: 5,
        refresh: time::Duration::seconds(2),
    };
    let l = Limiter::new(ctx, rate);

    // Immediately acquire `burst` permits one by one.
    let now = ctx.now();
    for _ in 0..rate.burst {
        l.acquire(ctx, 1).await.unwrap();
        assert_eq!(now, ctx.now());
    }

    // To acquire more permits wait for the refresh.
    for i in 0..rate.burst + 1 {
        let now = ctx.now();
        // Wait for a while before acquiring.
        ctx.sleep(rate.refresh * (i as u32) / 2).await.unwrap();
        l.acquire(ctx, i).await.unwrap();
        assert_eq!(now + (i as u32) * rate.refresh, ctx.now());
    }

    // After `x` refresh periods, you can immediately acquire
    // `x` permits.
    for x in 0..rate.burst + 1 {
        ctx.sleep(x as u32 * rate.refresh).await.unwrap();
        let now = ctx.now();
        for _ in 0..x {
            l.acquire(ctx, 1).await.unwrap();
            assert_eq!(now, ctx.now());
        }
        l.acquire(ctx, 1).await.unwrap();
        assert_eq!(now + rate.refresh, ctx.now());
    }
}

#[tokio::test]
async fn infinite_refresh_rate() {
    testonly::abort_on_panic();
    let clock = &ctx::ManualClock::new();
    let ctx = &ctx::test_root(clock);
    let rng = &mut ctx.rng();
    let rate = Rate {
        burst: 5,
        // Permits are refreshed immediately.
        // This should effectively disable rate limiting.
        refresh: time::Duration::ZERO,
    };
    let l = Limiter::new(ctx, rate);
    let now = ctx.now();
    for _ in 0..10 {
        l.acquire(ctx, rng.gen_range(0..=rate.burst)).await.unwrap();
    }
    assert_eq!(now, ctx.now());
}

#[tokio::test]
async fn delayed_permit_consumption() {
    testonly::abort_on_panic();
    let clock = &ctx::ManualClock::new();
    clock.set_advance_on_sleep();
    let ctx = &ctx::test_root(clock);
    let rng = &mut ctx.rng();
    let rate = Rate {
        burst: 20,
        refresh: time::Duration::seconds(1),
    };
    let l = Limiter::new(ctx, rate);

    // Reserve most of the permits.
    let permits = l.acquire(ctx, rate.burst - 1).await.unwrap();
    // Remaining permit should get refreshed as usual.
    l.acquire(ctx, 1).await.unwrap();
    for _ in 0..10 {
        let now = ctx.now();
        l.acquire(ctx, 1).await.unwrap();
        assert_eq!(now + rate.refresh, ctx.now());
    }
    drop(permits);

    // Reserved tokens can be refreshed only once they are consumed,
    // so acquiring more tokens may may require some reserved tokens
    // to get consumed first.
    for x in 0..rate.burst {
        let l = Limiter::new(ctx, rate);
        let delay = rng.gen_range(15..81) * time::Duration::SECOND;
        let permits = l.acquire(ctx, rate.burst - x).await.unwrap();
        let now = ctx.now();
        scope::run!(ctx, |ctx, s| async {
            s.spawn(async {
                // Drop permits after a delay.
                ctx.sleep(delay).await?;
                drop(permits);
                Ok(())
            });
            l.acquire(ctx, x + 1).await?;
            // Note that after waiting for a `delay`, `rate.burst-x` permits
            // are consumed and we need to wait for 1 of them to get refreshed
            // to reserve x+1 permits.
            assert_eq!(now + delay + rate.refresh, ctx.now());
            ctx::OrCanceled::Ok(())
        })
        .await
        .unwrap();
    }

    // Sometimes multiple permits need to be consumed before acquire
    // completes.
    let delay = (rate.burst as u32) * rate.refresh;
    let n = 5;
    let mut permits = vec![];
    for _ in 0..rate.burst {
        permits.push(l.acquire(ctx, 1).await.unwrap());
    }
    scope::run!(ctx, |ctx, s| async {
        let now = ctx.now();
        s.spawn(async {
            for _ in 0..n {
                ctx.sleep(delay).await.unwrap();
                permits.pop();
                // Allow the acquire() call to try to progress.
                // (best effort: `yield_now()` doesn't give strong guarantees).
                sync::yield_now().await;
            }
            Ok(())
        });
        l.acquire(ctx, n).await.unwrap();
        assert_eq!(now + (n as u32) * delay + rate.refresh, ctx.now());
        ctx::OrCanceled::Ok(())
    })
    .await
    .unwrap();
}
