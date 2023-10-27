use crate::{frame, mux, noise, noise::bytes};
use anyhow::Context as _;
use concurrency::{ctx, scope};
use rand::Rng as _;
use schema::proto::network::mux_test as proto;
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use utils::no_copy::NoCopy;

fn assert_partition(sets: &[u16]) {
    let mut sum = 0;
    for set in sets {
        assert_eq!(0, sum & set);
        sum |= set;
    }
    assert_eq!(u16::MAX, sum);
}

#[test]
fn test_masks() {
    assert_partition(&[
        mux::FrameKind::MASK,
        mux::StreamKind::MASK,
        mux::StreamId::MASK,
    ]);
}

#[test]
fn test_mux_verify() {
    let cfg = Arc::new(mux::Config {
        read_buffer_size: 1000,
        read_frame_size: 100,
        read_frame_count: 10,
        write_frame_size: 100,
    });
    assert!(mux::Mux {
        cfg: cfg.clone(),
        accept: [].into(),
        connect: [].into(),
    }
    .verify()
    .is_ok());

    let mut queues = BTreeMap::new();
    queues.insert(0, mux::StreamQueue::new(u32::MAX));
    queues.insert(1, mux::StreamQueue::new(u32::MAX));
    // Total streams overflow:
    assert!(mux::Mux {
        cfg: cfg.clone(),
        accept: queues.clone(),
        connect: [].into(),
    }
    .verify()
    .is_err());
    assert!(mux::Mux {
        cfg,
        accept: [].into(),
        connect: queues.clone(),
    }
    .verify()
    .is_err());
}

struct Req(Vec<u8>);
struct Resp {
    output: Vec<u8>,
    capability_id: mux::CapabilityId,
}

impl schema::ProtoFmt for Req {
    type Proto = proto::Req;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(r.input.as_ref().unwrap().clone()))
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            input: Some(self.0.clone()),
        }
    }
}

impl schema::ProtoFmt for Resp {
    type Proto = proto::Resp;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            output: r.output.as_ref().unwrap().clone(),
            capability_id: r.capability_id.unwrap(),
        })
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            output: Some(self.output.clone()),
            capability_id: Some(self.capability_id),
        }
    }
}

/// An arbitrarily chosen nontrivial transformation
/// which is used as an RPC in our test.
fn rpc_handler(mut req: Vec<u8>) -> Vec<u8> {
    req.reverse();
    req
}

/// Server handling multiple RPCs per transient stream.
/// Executes `rpc_handler` on each RPC request and sends back the response.
async fn run_server(
    ctx: &ctx::Ctx,
    queue: Arc<mux::StreamQueue>,
    cap: mux::CapabilityId,
) -> anyhow::Result<()> {
    let count = AtomicUsize::new(0);
    scope::run!(ctx, |ctx, s| async {
        while let Ok(stream) = queue.open(ctx).await {
            assert!(count.fetch_add(1, Ordering::SeqCst) < queue.max_streams as usize);
            s.spawn(async {
                let mut stream = stream;
                while let Ok((req, _)) = frame::mux_recv_proto::<Req>(ctx, &mut stream.read).await {
                    let resp = Resp {
                        output: rpc_handler(req.0),
                        capability_id: cap,
                    };
                    frame::mux_send_proto(ctx, &mut stream.write, &resp)
                        .await
                        .unwrap();
                    stream.write.flush(ctx).await.unwrap();
                }
                count.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            });
        }
        Ok(())
    })
    .await
}

async fn run_client(
    ctx: &ctx::Ctx,
    queue: Arc<mux::StreamQueue>,
    cap: mux::CapabilityId,
) -> anyhow::Result<()> {
    let rng = &mut ctx.rng();
    let count = AtomicUsize::new(0);
    let sessions: usize = rng.gen_range(10..20);
    scope::run!(ctx, |ctx, s| async {
        for _ in 0..sessions {
            let stream = queue.open(ctx).await?;
            assert!(count.fetch_add(1, Ordering::SeqCst) < queue.max_streams as usize);
            s.spawn(async {
                let rng = &mut ctx.rng();
                let mut stream = stream;
                let rpcs = rng.gen_range(3..10);
                for _ in 0..rpcs {
                    let mut req = vec![0u8; rng.gen_range(1000..2000)];
                    rng.fill(&mut req[..]);
                    frame::mux_send_proto(ctx, &mut stream.write, &Req(req.clone())).await?;
                    stream.write.flush(ctx).await?;
                    let (resp, _) = frame::mux_recv_proto::<Resp>(ctx, &mut stream.read).await?;
                    assert_eq!(resp.output, rpc_handler(req));
                    assert_eq!(resp.capability_id, cap);
                }
                count.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            });
        }
        Ok(())
    })
    .await
}

fn expected(res: Result<(), mux::RunError>) -> Result<(), mux::RunError> {
    match res {
        Err(mux::RunError::Closed | mux::RunError::Canceled(_)) => Ok(()),
        res => res,
    }
}

// This is full a large coverage test
// which checks bidirectional communication, in which
// * both ends act as both server and client
// * requires flushing (multiple messages are exchanged over each transient stream)
// * multiple capabilities are used at the same time.
// * ends use totally different configs
// * messages are larger than frames
//
// TODO(gprusak): in case the test fails it may be hard to find the actual bug, because
// this test covers a lot of features. In such situation more specific tests
// checking 1 property at a time should be added.
#[test]
fn mux_with_noise() {
    concurrency::testonly::abort_on_panic();
    concurrency::testonly::with_runtimes(|| async {
        let ctx = &ctx::test_root(&ctx::RealClock);
        let rng = &mut ctx.rng();

        let caps = 3;

        // Use totally different configs for each end.
        let mux1 = mux::Mux {
            cfg: Arc::new(mux::Config {
                read_buffer_size: 1000,
                read_frame_size: 100,
                read_frame_count: 7,
                write_frame_size: 150,
            }),
            accept: (0..caps)
                .map(|c| (c, mux::StreamQueue::new(rng.gen_range(1..5))))
                .collect(),
            connect: (0..caps)
                .map(|c| (c, mux::StreamQueue::new(rng.gen_range(1..5))))
                .collect(),
        };
        let mux2 = mux::Mux {
            cfg: Arc::new(mux::Config {
                read_buffer_size: 800,
                read_frame_size: 80,
                read_frame_count: 10,
                write_frame_size: 79,
            }),
            accept: (0..caps)
                .map(|c| (c, mux::StreamQueue::new(rng.gen_range(1..5))))
                .collect(),
            connect: (0..caps)
                .map(|c| (c, mux::StreamQueue::new(rng.gen_range(1..5))))
                .collect(),
        };

        // Different buffer size and frame count.
        assert!(mux1.cfg.read_buffer_size != mux2.cfg.read_buffer_size);
        assert!(mux1.cfg.read_frame_count != mux2.cfg.read_frame_count);

        // One side is using smaller frames than the other.
        assert!(mux1.cfg.read_frame_size > mux2.cfg.write_frame_size);
        assert!(mux2.cfg.read_frame_size < mux1.cfg.write_frame_size);

        scope::run!(ctx, |ctx, s| async {
            let (s1, s2) = noise::testonly::pipe(ctx).await;
            for (cap, q) in mux1.connect.clone() {
                let cap = NoCopy::from(cap);
                s.spawn_bg(async {
                    run_server(ctx, q, *cap)
                        .await
                        .with_context(|| format!("server({})", cap.into_inner()))
                });
            }
            for (cap, q) in mux1.accept.clone() {
                let cap = NoCopy::from(cap);
                s.spawn(async {
                    run_client(ctx, q, *cap)
                        .await
                        .with_context(|| format!("client({})", cap.into_inner()))
                });
            }
            for (cap, q) in mux2.connect.clone() {
                let cap = NoCopy::from(cap);
                s.spawn_bg(async {
                    run_server(ctx, q, *cap)
                        .await
                        .with_context(|| format!("server({})", cap.into_inner()))
                });
            }
            for (cap, q) in mux2.accept.clone() {
                let cap = NoCopy::from(cap);
                s.spawn(async {
                    run_client(ctx, q, *cap)
                        .await
                        .with_context(|| format!("client({})", cap.into_inner()))
                });
            }
            s.spawn_bg(async { expected(mux1.run(ctx, s1).await).context("mux1.run()") });
            s.spawn_bg(async { expected(mux2.run(ctx, s2).await).context("mux2.run()") });
            Ok(())
        })
        .await
        .unwrap();
    });
}

#[tokio::test]
async fn test_transport_closed() {
    concurrency::testonly::abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let cap: mux::CapabilityId = 0;
    let cfg = Arc::new(mux::Config {
        read_buffer_size: 1000,
        read_frame_size: 100,
        read_frame_count: 10,
        write_frame_size: 100,
    });
    scope::run!(ctx, |ctx, s| async {
        let streams = s.spawn(async {
            let (s1, s2) = noise::testonly::pipe(ctx).await;
            // Establish a transient stream, then close the transport (s1 and s2).
            scope::run!(ctx, |ctx, s| async {
                let outbound = s.spawn(async {
                    let mut mux = mux::Mux {
                        cfg: cfg.clone(),
                        accept: BTreeMap::default(),
                        connect: BTreeMap::default(),
                    };
                    let q = mux::StreamQueue::new(1);
                    mux.connect.insert(cap, q.clone());
                    s.spawn_bg(async { expected(mux.run(ctx, s2).await).context("[connect] mux.run()") });
                    q.open(ctx).await.context("[connect] q.open()")
                });
                let inbound = s.spawn(async {
                    let mut mux = mux::Mux {
                        cfg: cfg.clone(),
                        accept: BTreeMap::default(),
                        connect: BTreeMap::default(),
                    };
                    let q = mux::StreamQueue::new(1);
                    mux.accept.insert(cap, q.clone());
                    s.spawn_bg(async { expected(mux.run(ctx, s1).await).context("[accept] mux.run()") });
                    q.open(ctx).await.context("[accept] q.open()")
                });
                Ok([
                   inbound.join(ctx).await.context("inbound")?,
                   outbound.join(ctx).await.context("outbound")?,
                ])
            }).await
        }).join(ctx).await?;
        // Check how the streams without transport behave.
        for mut s in streams {
            let mut buf = bytes::Buffer::new(100);
            // Read is expected to succeed, but no data should be read.
            s.read.read_exact(ctx, &mut buf).await.unwrap();
            assert_eq!(buf.len(), 0);
            // Writing will succeed (thanks to buffering), but flushing should fail
            // because the transport is closed.
            s.write.write_all(ctx, &[1, 2, 3]).await.unwrap();
            assert!(s.write.flush(ctx).await.is_err());
        }
        Ok(())
    })
    .await
    .unwrap();
}
