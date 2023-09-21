use super::*;
use crate::{frame, noise, testonly};
use concurrency::{ctx, io, scope};
use rand::Rng;
use roles::node;
use std::collections::{HashMap, HashSet};

#[test]
fn test_schema_encode_decode() {
    let rng = &mut ctx::test_root(&ctx::RealClock).rng();
    schema::testonly::test_encode_random::<_, Handshake>(rng);
}

fn make_cfg<R: Rng>(rng: &mut R) -> Config {
    Config {
        key: rng.gen(),
        dynamic_inbound_limit: 0,
        static_inbound: HashSet::default(),
        static_outbound: HashMap::default(),
        enable_pings: true,
    }
}

#[tokio::test]
async fn test_session_id_mismatch() {
    concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let cfg0 = make_cfg(rng);
    let cfg1 = make_cfg(rng);

    // MitM attempt detected on the inbound end.
    scope::run!(ctx, |ctx, s| async {
        let (s1, s2) = noise::testonly::pipe(ctx).await;
        let (s3, s4) = noise::testonly::pipe(ctx).await;
        let (r2, w2) = io::split(s2);
        let (r3, w3) = io::split(s3);
        s.spawn_bg(async {
            testonly::forward(ctx, r2, w3).await;
            Ok(())
        });
        s.spawn_bg(async {
            testonly::forward(ctx, r3, w2).await;
            Ok(())
        });
        s.spawn(async {
            let mut s4 = s4;
            match inbound(ctx, &cfg0, &mut s4).await {
                Err(Error::SessionIdMismatch) => Ok(()),
                res => panic!("unexpected res: {res:?}"),
            }
        });
        s.spawn(async {
            let mut s1 = s1;
            match outbound(ctx, &cfg1, &mut s1, &cfg0.key.public()).await {
                Err(Error::Stream(..)) => Ok(()),
                res => panic!("unexpected res: {res:?}"),
            }
        });
        anyhow::Ok(())
    })
    .await
    .unwrap();

    // MitM attempt detected on the outbound end.
    scope::run!(ctx, |ctx, s| async {
        let (mut s1, s2) = noise::testonly::pipe(ctx).await;
        s.spawn_bg(async {
            let mut s2 = s2;
            let _: Handshake = frame::recv_proto(ctx, &mut s2).await?;
            frame::send_proto(
                ctx,
                &mut s2,
                &Handshake {
                    session_id: cfg1.key.sign_msg(rng.gen::<node::SessionId>()),
                    is_static: false,
                },
            )
            .await?;
            Ok(())
        });
        match outbound(ctx, &cfg0, &mut s1, &cfg1.key.public()).await {
            Err(Error::SessionIdMismatch) => anyhow::Ok(()),
            res => panic!("unexpected res: {res:?}"),
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_peer_mismatch() {
    concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let cfg0 = make_cfg(rng);
    let cfg1 = make_cfg(rng);
    let cfg2 = make_cfg(rng);

    scope::run!(ctx, |ctx, s| async {
        let (s0, s1) = noise::testonly::pipe(ctx).await;
        s.spawn(async {
            let mut s0 = s0;
            assert_eq!(cfg1.key.public(), inbound(ctx, &cfg0, &mut s0).await?);
            Ok(())
        });
        s.spawn(async {
            let mut s1 = s1;
            match outbound(ctx, &cfg1, &mut s1, &cfg2.key.public()).await {
                Err(Error::PeerMismatch) => Ok(()),
                res => panic!("unexpected res: {res:?}"),
            }
        });
        anyhow::Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_invalid_signature() {
    concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let cfg0 = make_cfg(rng);
    let cfg1 = make_cfg(rng);

    // Bad signature detected on outbound end.
    scope::run!(ctx, |ctx, s| async {
        let (mut s0, s1) = noise::testonly::pipe(ctx).await;
        s.spawn_bg(async {
            let mut s1 = s1;
            let mut h: Handshake = frame::recv_proto(ctx, &mut s1).await?;
            h.session_id.key = cfg1.key.public();
            frame::send_proto(ctx, &mut s1, &h).await?;
            Ok(())
        });
        match outbound(ctx, &cfg0, &mut s0, &cfg1.key.public()).await {
            Err(Error::Signature(..)) => anyhow::Ok(()),
            res => panic!("unexpected res: {res:?}"),
        }
    })
    .await
    .unwrap();

    // Bad signature detected on inbound end.
    scope::run!(ctx, |ctx, s| async {
        let (mut s0, s1) = noise::testonly::pipe(ctx).await;
        s.spawn_bg(async {
            let mut s1 = s1;
            let mut h = Handshake {
                session_id: cfg0.key.sign_msg(node::SessionId(s1.id().encode())),
                is_static: true,
            };
            h.session_id.key = cfg1.key.public();
            frame::send_proto(ctx, &mut s1, &h).await
        });
        match inbound(ctx, &cfg0, &mut s0).await {
            Err(Error::Signature(..)) => anyhow::Ok(()),
            res => panic!("unexpected res: {res:?}"),
        }
    })
    .await
    .unwrap();
}
