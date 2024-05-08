use super::*;
use crate::noise;
use rand::Rng as _;
use std::{
    collections::HashSet,
    sync::atomic::{AtomicU64, Ordering},
};
use zksync_concurrency::{ctx, testonly::abort_on_panic, time};
use zksync_protobuf::{kB, testonly::test_encode_random};

/// CAPABILITY_ID should uniquely identify the RPC.
#[test]
fn test_capability_rpc_correspondence() {
    let ids = [
        consensus::Rpc::CAPABILITY_ID,
        push_validator_addrs::Rpc::CAPABILITY_ID,
        push_block_store_state::Rpc::CAPABILITY_ID,
        get_block::Rpc::CAPABILITY_ID,
        ping::Rpc::CAPABILITY_ID,
        push_batch_signature::Rpc::CAPABILITY_ID,
    ];
    assert_eq!(ids.len(), HashSet::from(ids).len());
}

#[test]
fn test_schema_encode_decode() {
    let rng = &mut ctx::test_root(&ctx::RealClock).rng();
    test_encode_random::<consensus::Req>(rng);
    test_encode_random::<consensus::Resp>(rng);
    test_encode_random::<push_validator_addrs::Req>(rng);
    test_encode_random::<push_block_store_state::Req>(rng);
    test_encode_random::<get_block::Req>(rng);
    test_encode_random::<get_block::Resp>(rng);
}

fn expected(res: Result<(), mux::RunError>) -> Result<(), mux::RunError> {
    match res {
        Err(mux::RunError::Closed | mux::RunError::Canceled(_)) => Ok(()),
        res => res,
    }
}

#[tokio::test]
async fn test_ping() {
    abort_on_panic();
    let clock = ctx::ManualClock::new();
    let ctx = &ctx::test_root(&clock);
    let (s1, s2) = noise::testonly::pipe(ctx).await;
    let client = Client::<ping::Rpc>::new(ctx, ping::RATE);
    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(async {
            expected(
                Service::new()
                    .add_server(ctx, ping::Server, ping::RATE)
                    .run(ctx, s1)
                    .await,
            )
            .context("server")
        });
        s.spawn_bg(async {
            expected(Service::new().add_client(&client).run(ctx, s2).await).context("client")
        });
        for _ in 0..ping::RATE.burst {
            let req = ping::Req(ctx.rng().gen());
            let resp = client.call(ctx, &req, kB).await?;
            assert_eq!(req.0, resp.0);
        }
        clock.advance(ping::RATE.refresh);
        let req = ping::Req(ctx.rng().gen());
        let resp = client.call(ctx, &req, kB).await?;
        assert_eq!(req.0, resp.0);
        Ok(())
    })
    .await
    .unwrap();
}

struct PingServer {
    clock: ctx::ManualClock,
    pings: AtomicU64,
}

const PING_COUNT: u64 = 3;
const PING_TIMEOUT: time::Duration = time::Duration::seconds(6);

#[async_trait::async_trait]
impl Handler<ping::Rpc> for PingServer {
    fn max_req_size(&self) -> usize {
        kB
    }
    async fn handle(&self, ctx: &ctx::Ctx, req: ping::Req) -> anyhow::Result<ping::Resp> {
        if self.pings.fetch_add(1, Ordering::Relaxed) >= PING_COUNT {
            self.clock.advance(PING_TIMEOUT);
            ctx.canceled().await;
            Err(ctx::Canceled.into())
        } else {
            Ok(ping::Resp(req.0))
        }
    }
}

#[tokio::test]
async fn test_ping_loop() {
    abort_on_panic();
    let clock = ctx::ManualClock::new();
    clock.set_advance_on_sleep();
    let ctx = &ctx::test_root(&clock);
    let (s1, s2) = noise::testonly::pipe(ctx).await;
    let client = Client::<ping::Rpc>::new(ctx, ping::RATE);
    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(async {
            // Clock is passed to the server, so that it can
            // timeout the ping request at the right moment.
            let server = PingServer {
                clock,
                pings: 0.into(),
            };

            expected(
                Service::new()
                    .add_server(
                        ctx,
                        server,
                        limiter::Rate {
                            burst: 1,
                            // with `refresh = 0`, server will never autoadvance time.
                            refresh: time::Duration::ZERO,
                        },
                    )
                    .run(ctx, s1)
                    .await,
            )
            .context("server")
        });
        s.spawn_bg(async {
            expected(Service::new().add_client(&client).run(ctx, s2).await).context("client")
        });
        let now = ctx.now();
        assert!(client.ping_loop(ctx, PING_TIMEOUT).await.is_err());
        let got = ctx.now() - now;
        // PING_COUNT will succeed and the next with time out.
        let want = (PING_COUNT + 1) as u32 * PING_TIMEOUT;
        assert_eq!(got, want);
        Ok(())
    })
    .await
    .unwrap();
}

struct ExampleRpc;

const RATE: limiter::Rate = limiter::Rate {
    burst: 10,
    refresh: time::Duration::ZERO,
};

impl Rpc for ExampleRpc {
    const CAPABILITY_ID: mux::CapabilityId = 0;
    const INFLIGHT: u32 = 5;
    const METHOD: &'static str = "example";
    type Req = ();
    type Resp = ();
}

struct ExampleServer;

#[async_trait::async_trait]
impl Handler<ExampleRpc> for ExampleServer {
    fn max_req_size(&self) -> usize {
        kB
    }
    async fn handle(&self, ctx: &ctx::Ctx, _req: ()) -> anyhow::Result<()> {
        ctx.canceled().await;
        anyhow::bail!("terminated");
    }
}

#[tokio::test]
async fn test_inflight() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::RealClock);
    let (s1, s2) = noise::testonly::pipe(ctx).await;
    let client = Client::<ExampleRpc>::new(ctx, RATE);
    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(async {
            expected(
                Service::new()
                    .add_server(ctx, ExampleServer, RATE)
                    .run(ctx, s1)
                    .await,
            )
            .context("server")
        });
        s.spawn_bg(async {
            expected(Service::new().add_client(&client).run(ctx, s2).await).context("client")
        });
        let mut calls = vec![];
        // It should be possible to reserve INFLIGHT calls before executing
        // any of them.
        for _ in 0..ExampleRpc::INFLIGHT {
            calls.push(client.reserve(ctx).await?);
        }
        anyhow::Ok(())
    })
    .await
    .unwrap();
}
