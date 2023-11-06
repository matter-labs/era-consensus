use crate::noise;
use rand::Rng;
use tracing::instrument::Instrument as _;
use zksync_concurrency as concurrency;
use zksync_concurrency::{ctx, io, scope};

#[tokio::test]
async fn transmit_ok() {
    concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let (mut s1, mut s2) = noise::testonly::pipe(ctx).await;
    let msg = "hello";
    let n = 10;
    scope::run!(ctx, |ctx, s| async {
        s.spawn(
            async {
                let mut got = vec![0; msg.len()];
                for _ in 0..n {
                    io::read_exact(ctx, &mut s1, &mut got).await??;
                    assert_eq!(&got, msg.as_bytes());
                }
                io::shutdown(ctx, &mut s1).await??;
                Ok(())
            }
            .instrument(tracing::info_span!("server")),
        );
        s.spawn(
            async {
                for i in 0..n {
                    io::write_all(ctx, &mut s2, msg.as_bytes()).await??;
                    if i % 3 == 0 {
                        io::flush(ctx, &mut s2).await??;
                    }
                }
                io::shutdown(ctx, &mut s2).await??;
                Ok(())
            }
            .instrument(tracing::info_span!("client")),
        );
        anyhow::Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn transmit_sender_dies() {
    concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let (mut s1, mut s2) = noise::testonly::pipe(ctx).await;
    let msg = "hello";
    scope::run!(ctx, |ctx, s| async {
        s.spawn(
            async {
                let mut got = vec![0; msg.len() + 1];
                assert!(io::read_exact(ctx, &mut s1, &mut got).await?.is_err());
                io::shutdown(ctx, &mut s1).await??;
                Ok(())
            }
            .instrument(tracing::info_span!("server")),
        );
        s.spawn(
            async {
                io::write_all(ctx, &mut s2, msg.as_bytes()).await??;
                io::shutdown(ctx, &mut s2).await??;
                Ok(())
            }
            .instrument(tracing::info_span!("client")),
        );
        anyhow::Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn transmit_receiver_dies() {
    concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let (s1, s2) = noise::testonly::pipe(ctx).await;
    let mut msg = [0u8; 1024];
    rng.fill(&mut msg);

    scope::run!(ctx, |ctx, s| async {
        s.spawn(
            async {
                let mut s1 = s1;
                let mut got = vec![0; msg.len()];
                io::read_exact(ctx, &mut s1, &mut got).await??;
                assert_eq!(&got, &msg);
                Ok(())
            }
            .instrument(tracing::info_span!("server")),
        );
        s.spawn(
            async {
                let mut s2 = s2;
                while io::write_all(ctx, &mut s2, &msg).await?.is_ok() {}
                Ok(())
            }
            .instrument(tracing::info_span!("client")),
        );
        anyhow::Ok(())
    })
    .await
    .unwrap();
}
