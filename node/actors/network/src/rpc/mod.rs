//! Generic RPC service built on top of the multiplexer.
//! To define a new Rpc define a new type which implements `Rpc` trait.
//! Each RPC type has a unique `mux::CapabilityId`.
//! To implement a server for the given RPC type `X` implement `Handler<X>`.
//! To run an RPC server on a tcp stream:
//! ```ignore
//! let server = <type implementing Handler<X>>;
//! Service::new().add_server(server).run(ctx,stream).await;
//! ```
//! To establish a connection to an Rpc server over a tcp stream:
//! ```ignore
//! let client = Client::<X>::new();
//! Service::new().add_client(&client).run(ctx,stream).await;
//! ```
//! You can construct an Rpc service with multiple servers and clients
//! at the same time (max 1 client + server per CapabilityId).
use crate::{frame, mux};
use anyhow::Context as _;
use concurrency::{ctx, io, limiter, metrics, scope};
use once_cell::sync::Lazy;
use std::{collections::BTreeMap, sync::Arc};

pub(crate) mod consensus;
pub(crate) mod ping;
pub(crate) mod sync_blocks;
pub(crate) mod sync_validator_addrs;
#[cfg(test)]
pub(crate) mod testonly;
#[cfg(test)]
mod tests;

static RPC_LATENCY: Lazy<prometheus::HistogramVec> = Lazy::new(|| {
    prometheus::register_histogram_vec!(
        "network_rpc_latency",
        "latency of Rpcs in seconds",
        &["type", "method", "submethod", "result"],
        prometheus::exponential_buckets(0.01, 1.5, 20).unwrap(),
    )
    .unwrap()
});
static RPC_INFLIGHT: Lazy<prometheus::IntGaugeVec> = Lazy::new(|| {
    prometheus::register_int_gauge_vec!(
        "network_rpc_inflight",
        "Rpcs inflight",
        &["type", "method", "submethod"]
    )
    .unwrap()
});
static RPC_MESSAGE_SIZE: Lazy<prometheus::HistogramVec> = Lazy::new(|| {
    prometheus::register_histogram_vec!(
        "network_rpc_message_size",
        "message sizes in bytes",
        &["type", "method", "submethod"],
    )
    .unwrap()
});
static RPC_CALL_RESERVE_LATENCY: Lazy<prometheus::HistogramVec> = Lazy::new(|| {
    prometheus::register_histogram_vec!(
        "network_rpc_call_reserve_latency",
        "time that client waits for the server to prepare a stream for an RPC call",
        &["method"],
    )
    .unwrap()
});

const CLIENT_SEND_RECV: &str = "client_send_recv";
const SERVER_RECV_SEND: &str = "server_recv_send";
const SERVER_PROCESS: &str = "server_process";

const REQ_SENT: &str = "req_sent";
const REQ_RECV: &str = "req_recv";
const RESP_SENT: &str = "resp_sent";
const RESP_RECV: &str = "resp_recv";

const OK: &str = "ok";
const ERR: &str = "err";

const MUX_CONFIG: mux::Config = mux::Config {
    read_buffer_size: 160 * schema::kB as u64,
    read_frame_size: 16 * schema::kB as u64,
    read_frame_count: 100,
    write_frame_size: 16 * schema::kB as u64,
};

/// Trait for defining an RPC.
/// It is just a collection of associated types
/// and constants.
pub(crate) trait Rpc: Sync + Send + 'static {
    /// CapabilityId used to identify the RPC.
    /// Client and Server have to agree on CAPABILITY_ID
    /// of all supported RPC, for the RPC to actually work.
    /// Each type implementing `Rpc` should use an unique
    /// `CAPABILITY_ID`.
    const CAPABILITY_ID: mux::CapabilityId;
    /// Maximal number of calls executed in parallel.
    /// Both client and server enforce this limit.
    const INFLIGHT: u32;
    /// Maximal rate at which calls can be made.
    /// Both client and server enforce this limit.
    const RATE: limiter::Rate;
    /// Name of the RPC, used in prometheus metrics.
    const METHOD: &'static str;
    /// Type of the request message.
    type Req: schema::ProtoFmt + Send + Sync;
    /// Type of the response message.
    type Resp: schema::ProtoFmt + Send + Sync;
    /// Name of the variant of the request message type.
    /// Useful for collecting metrics with finer than
    /// per-rpc type granularity.
    fn submethod(_req: &Self::Req) -> &'static str {
        ""
    }
}

/// Represents server's capacity to immediately handle 1 RPC call.
/// If you are connected to multiple servers, you can pool ReservedCalls
/// for maximum throughput - without ReservedCalls you would have to
/// blindly decide which server to call without knowing their real capacity.
/// TODO(gprusak): to actually pass around the permit, we should use an OwnedPermit
/// instead.
pub(crate) struct ReservedCall<'a, R: Rpc> {
    stream: mux::ReservedStream,
    permit: limiter::Permit<'a>,
    _rpc: std::marker::PhantomData<R>,
}

impl<'a, R: Rpc> ReservedCall<'a, R> {
    /// Performs the call.
    pub(crate) async fn call(self, ctx: &ctx::Ctx, req: &R::Req) -> anyhow::Result<R::Resp> {
        let send_time = ctx.now();
        let mut stream = self.stream.open(ctx).await??;
        drop(self.permit);
        let res = async {
            let _guard = metrics::GaugeGuard::from(RPC_INFLIGHT.with_label_values(&[
                "client",
                R::submethod(req),
                R::METHOD,
            ]));
            let msg_size = frame::mux_send_proto(ctx, &mut stream.write, req).await?;
            RPC_MESSAGE_SIZE
                .with_label_values(&[REQ_SENT, R::METHOD, R::submethod(req)])
                .observe(msg_size as f64);
            drop(stream.write);
            frame::mux_recv_proto(ctx, &mut stream.read).await
        }
        .await;
        let res_label = match res {
            Ok(_) => OK,
            Err(_) => ERR,
        };
        let now = ctx.now();
        RPC_LATENCY
            .with_label_values(&[CLIENT_SEND_RECV, R::METHOD, R::submethod(req), res_label])
            .observe((now - send_time).as_seconds_f64());
        let (res, msg_size) = res?;
        RPC_MESSAGE_SIZE
            .with_label_values(&[RESP_RECV, R::METHOD, R::submethod(req)])
            .observe(msg_size as f64);
        Ok(res)
    }
}

/// RPC client used to issue the calls to the server.
pub(crate) struct Client<R: Rpc> {
    limiter: limiter::Limiter,
    queue: Arc<mux::StreamQueue>,
    _rpc: std::marker::PhantomData<R>,
}

impl<R: Rpc> Client<R> {
    /// Constructs a new client.
    pub(crate) fn new(ctx: &ctx::Ctx) -> Self {
        Client {
            limiter: limiter::Limiter::new(ctx, R::RATE),
            queue: mux::StreamQueue::new(R::INFLIGHT),
            _rpc: std::marker::PhantomData,
        }
    }

    /// Reserves an RPC.
    pub(crate) async fn reserve<'a>(
        &'a self,
        ctx: &'a ctx::Ctx,
    ) -> anyhow::Result<ReservedCall<'a, R>> {
        let reserve_time = ctx.now();
        let permit = self.limiter.acquire(ctx, 1).await.context("limiter")?;
        let stream = self
            .queue
            .reserve(ctx)
            .await
            .context("StreamQueue::open()")?;
        RPC_CALL_RESERVE_LATENCY
            .with_label_values(&[R::METHOD])
            .observe((ctx.now() - reserve_time).as_seconds_f64());
        Ok(ReservedCall {
            stream,
            permit,
            _rpc: std::marker::PhantomData,
        })
    }

    /// Performs an RPC.
    pub(crate) async fn call(&self, ctx: &ctx::Ctx, req: &R::Req) -> anyhow::Result<R::Resp> {
        self.reserve(ctx).await?.call(ctx, req).await
    }
}

/// Trait for defining RPC server implementations.
#[async_trait::async_trait]
pub(crate) trait Handler<R: Rpc>: Sync + Send {
    /// Processes the request and returns the response.
    async fn handle(&self, ctx: &ctx::Ctx, req: R::Req) -> anyhow::Result<R::Resp>;
}

/// Internal: an RPC server which wraps the Handler.
struct Server<R: Rpc, H: Handler<R>> {
    handler: H,
    queue: Arc<mux::StreamQueue>,
    _rpc: std::marker::PhantomData<R>,
}

#[async_trait::async_trait]
trait ServerTrait: Sync + Send {
    async fn serve(&self, ctx: &ctx::Ctx) -> ctx::OrCanceled<()>;
}

#[async_trait::async_trait]
impl<R: Rpc, H: Handler<R>> ServerTrait for Server<R, H> {
    /// Serves the incoming RPCs, respecting the rate limit and
    /// max inflight limit.
    async fn serve(&self, ctx: &ctx::Ctx) -> ctx::OrCanceled<()> {
        let limiter = limiter::Limiter::new(ctx, R::RATE);
        scope::run!(ctx, |ctx, s| async {
            for _ in 0..R::INFLIGHT {
                s.spawn::<()>(async {
                    loop {
                        let permit = limiter.acquire(ctx, 1).await?;
                        let mut stream = self.queue.open(ctx).await?;
                        drop(permit);
                        let res = async {
                            let recv_time = ctx.now();
                            let (req, msg_size) =
                                frame::mux_recv_proto::<R::Req>(ctx, &mut stream.read).await?;
                            let submethod = R::submethod(&req);
                            RPC_MESSAGE_SIZE
                                .with_label_values(&[REQ_RECV, R::METHOD, submethod])
                                .observe(msg_size as f64);
                            let _guard =
                                metrics::GaugeGuard::from(RPC_INFLIGHT.with_label_values(&[
                                    "server",
                                    R::METHOD,
                                    submethod,
                                ]));
                            let process_time = ctx.now();
                            let res = self.handler.handle(ctx, req).await.context(R::METHOD);
                            let res_label = match res {
                                Ok(_) => OK,
                                Err(_) => ERR,
                            };
                            RPC_LATENCY
                                .with_label_values(&[
                                    SERVER_PROCESS,
                                    R::METHOD,
                                    submethod,
                                    res_label,
                                ])
                                .observe((ctx.now() - process_time).as_seconds_f64());
                            let res = frame::mux_send_proto(ctx, &mut stream.write, &res?).await;
                            RPC_LATENCY
                                .with_label_values(&[
                                    SERVER_RECV_SEND,
                                    R::METHOD,
                                    submethod,
                                    res_label,
                                ])
                                .observe((ctx.now() - recv_time).as_seconds_f64());
                            let msg_size = res?;
                            RPC_MESSAGE_SIZE
                                .with_label_values(&[RESP_SENT, R::METHOD, submethod])
                                .observe(msg_size as f64);
                            anyhow::Ok(())
                        }
                        .await;
                        if let Err(err) = res {
                            tracing::info!("{err:#}");
                        }
                    }
                });
            }
            Ok(())
        })
        .await
    }
}

/// A collection of RPC service and clients
/// which should communicate over the given connection.
pub(crate) struct Service<'a> {
    mux: mux::Mux,
    servers: Vec<Box<dyn 'a + ServerTrait>>,
}

impl<'a> Service<'a> {
    /// Constructs a new RPC service.
    pub(crate) fn new() -> Self {
        Self {
            mux: mux::Mux {
                cfg: Arc::new(MUX_CONFIG.clone()),
                accept: BTreeMap::default(),
                connect: BTreeMap::default(),
            },
            servers: vec![],
        }
    }

    /// Adds a client to the RPC service.
    pub(crate) fn add_client<R: Rpc>(mut self, client: &Client<R>) -> Self {
        if self
            .mux
            .accept
            .insert(R::CAPABILITY_ID, client.queue.clone())
            .is_some()
        {
            panic!(
                "client for capability {} already registered",
                R::CAPABILITY_ID
            );
        }
        self
    }

    /// Adds a server to the RPC service.
    pub(crate) fn add_server<R: Rpc>(mut self, handler: impl Handler<R> + 'a) -> Self {
        let queue = mux::StreamQueue::new(R::INFLIGHT);
        if self
            .mux
            .connect
            .insert(R::CAPABILITY_ID, queue.clone())
            .is_some()
        {
            panic!(
                "server for capability {} already registered",
                R::CAPABILITY_ID
            );
        }
        self.servers.push(Box::new(Server {
            handler,
            queue,
            _rpc: std::marker::PhantomData,
        }));
        self
    }

    /// Runs the RPCs service over the provided transport stream.
    pub(crate) async fn run<S: io::AsyncRead + io::AsyncWrite + Send>(
        self,
        ctx: &ctx::Ctx,
        transport: S,
    ) -> Result<(), mux::RunError> {
        scope::run!(ctx, |ctx, s| async {
            for server in &self.servers {
                s.spawn(async { Ok(server.serve(ctx).await?) });
            }
            self.mux.run(ctx, transport).await
        })
        .await
    }
}
