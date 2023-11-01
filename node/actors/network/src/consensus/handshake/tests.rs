use super::*;
use crate::{frame, noise, testonly};
use concurrency::{ctx, io, scope};
use rand::Rng;
use roles::validator;

#[test]
fn test_schema_encode_decode() {
    let rng = &mut ctx::test_root(&ctx::RealClock).rng();
    protobuf::testonly::test_encode_random::<_, Handshake>(rng);
}

#[tokio::test]
async fn test_session_id_mismatch() {
    concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let key0: validator::SecretKey = rng.gen();
    let key1: validator::SecretKey = rng.gen();

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
            match inbound(ctx, &key0, &mut s4).await {
                Err(Error::SessionIdMismatch) => Ok(()),
                res => panic!("unexpected res: {res:?}"),
            }
        });
        s.spawn(async {
            let mut s1 = s1;
            match outbound(ctx, &key1, &mut s1, &key0.public()).await {
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
                    session_id: key1.sign_msg(rng.gen::<node::SessionId>()),
                },
            )
            .await?;
            Ok(())
        });
        match outbound(ctx, &key0, &mut s1, &key1.public()).await {
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

    let key0: validator::SecretKey = rng.gen();
    let key1: validator::SecretKey = rng.gen();
    let key2: validator::SecretKey = rng.gen();

    scope::run!(ctx, |ctx, s| async {
        let (s0, s1) = noise::testonly::pipe(ctx).await;
        s.spawn(async {
            let mut s0 = s0;
            assert_eq!(key1.public(), inbound(ctx, &key0, &mut s0).await?);
            Ok(())
        });
        s.spawn(async {
            let mut s1 = s1;
            match outbound(ctx, &key1, &mut s1, &key2.public()).await {
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

    let key0: validator::SecretKey = rng.gen();
    let key1: validator::SecretKey = rng.gen();

    // Bad signature detected on outbound end.
    scope::run!(ctx, |ctx, s| async {
        let (mut s0, s1) = noise::testonly::pipe(ctx).await;
        s.spawn_bg(async {
            let mut s1 = s1;
            let mut h: Handshake = frame::recv_proto(ctx, &mut s1).await?;
            h.session_id.key = key1.public();
            frame::send_proto(ctx, &mut s1, &h).await?;
            Ok(())
        });
        match outbound(ctx, &key0, &mut s0, &key1.public()).await {
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
                session_id: key0.sign_msg(node::SessionId(s1.id().encode())),
            };
            h.session_id.key = key1.public();
            frame::send_proto(ctx, &mut s1, &h).await
        });
        match inbound(ctx, &key0, &mut s0).await {
            Err(Error::Signature(..)) => anyhow::Ok(()),
            res => panic!("unexpected res: {res:?}"),
        }
    })
    .await
    .unwrap();
}
