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

use self::metrics::{CallLatencyType, CallType, RPC_METRICS};
use crate::{frame, mux};
use anyhow::Context as _;
use std::{collections::BTreeMap, sync::Arc};
use zksync_concurrency::{ctx, io, limiter, metrics::LatencyHistogramExt as _, scope};

pub(crate) mod consensus;
pub(crate) mod get_block;
mod metrics;
pub(crate) mod ping;
pub(crate) mod push_block_store_state;
pub(crate) mod push_validator_addrs;
#[cfg(test)]
pub(crate) mod testonly;
#[cfg(test)]
mod tests;

const MUX_CONFIG: mux::Config = mux::Config {
    read_buffer_size: 160 * zksync_protobuf::kB as u64,
    read_frame_size: 16 * zksync_protobuf::kB as u64,
    read_frame_count: 100,
    write_frame_size: 16 * zksync_protobuf::kB as u64,
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
    /// Name of the RPC, used in prometheus metrics.
    const METHOD: &'static str;
    /// Type of the request message.
    type Req: zksync_protobuf::ProtoFmt + Send + Sync;
    /// Type of the response message.
    type Resp: zksync_protobuf::ProtoFmt + Send + Sync;
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
    pub(crate) async fn call(
        self,
        ctx: &ctx::Ctx,
        req: &R::Req,
        max_resp_size: usize,
    ) -> anyhow::Result<R::Resp> {
        let send_time = ctx.now();
        let mut stream = self.stream.open(ctx).await??;
        drop(self.permit);
        let res = async {
            let metric_labels = CallType::Client.to_labels::<R>(req);
            let _guard = RPC_METRICS.inflight[&metric_labels].inc_guard(1);
            let msg_size = frame::mux_send_proto(ctx, &mut stream.write, req)
                .await
                .context("mux_send_proto(req)")?;
            RPC_METRICS.message_size[&CallType::ReqSent.to_labels::<R>(req)].observe(msg_size);
            drop(stream.write);
            frame::mux_recv_proto(ctx, &mut stream.read, max_resp_size).await
        }
        .await;

        let now = ctx.now();
        let metric_labels = CallLatencyType::ClientSendRecv.to_labels::<R>(req, &res);
        RPC_METRICS.latency[&metric_labels].observe_latency(now - send_time);
        let (res, msg_size) = res.context(R::METHOD)?;
        RPC_METRICS.message_size[&CallType::RespRecv.to_labels::<R>(req)].observe(msg_size);
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
    // TODO(gprusak): at this point we don't need the clients to be reusable,
    // so perhaps they should be constructed by `Service::add_client` instead?
    pub(crate) fn new(ctx: &ctx::Ctx, rate: limiter::Rate) -> Self {
        Client {
            limiter: limiter::Limiter::new(ctx, rate),
            queue: mux::StreamQueue::new(R::INFLIGHT),
            _rpc: std::marker::PhantomData,
        }
    }

    /// Reserves an RPC.
    pub(crate) async fn reserve<'a>(
        &'a self,
        ctx: &'a ctx::Ctx,
    ) -> ctx::OrCanceled<ReservedCall<'a, R>> {
        let reserve_time = ctx.now();
        let permit = self.limiter.acquire(ctx, 1).await?;
        let stream = self.queue.reserve(ctx).await?;
        RPC_METRICS.call_reserve_latency[&R::METHOD].observe_latency(ctx.now() - reserve_time);
        Ok(ReservedCall {
            stream,
            permit,
            _rpc: std::marker::PhantomData,
        })
    }

    /// Performs an RPC.
    pub(crate) async fn call(
        &self,
        ctx: &ctx::Ctx,
        req: &R::Req,
        max_resp_size: usize,
    ) -> ctx::Result<R::Resp> {
        Ok(self
            .reserve(ctx)
            .await?
            .call(ctx, req, max_resp_size)
            .await?)
    }
}

/// Trait for defining RPC server implementations.
#[async_trait::async_trait]
pub(crate) trait Handler<R: Rpc>: Sync + Send {
    /// Processes the request and returns the response.
    async fn handle(&self, ctx: &ctx::Ctx, req: R::Req) -> anyhow::Result<R::Resp>;
    /// Upper bound on the proto-encoded request size.
    /// It protects us from buffering maliciously large messages.
    fn max_req_size(&self) -> usize;
}

/// Internal: an RPC server which wraps the Handler.
struct Server<R: Rpc, H: Handler<R>> {
    handler: H,
    queue: Arc<mux::StreamQueue>,
    rate: limiter::Rate,
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
        let limiter = limiter::Limiter::new(ctx, self.rate);
        scope::run!(ctx, |ctx, s| async {
            for _ in 0..R::INFLIGHT {
                s.spawn::<()>(async {
                    loop {
                        let permit = limiter.acquire(ctx, 1).await?;
                        let mut stream = self.queue.open(ctx).await?;
                        drop(permit);
                        let res = async {
                            let recv_time = ctx.now();
                            let (req, msg_size) = frame::mux_recv_proto::<R::Req>(
                                ctx,
                                &mut stream.read,
                                self.handler.max_req_size(),
                            )
                            .await?;

                            let size_labels = CallType::ReqRecv.to_labels::<R>(&req);
                            let resp_size_labels = CallType::RespSent.to_labels::<R>(&req);
                            RPC_METRICS.message_size[&size_labels].observe(msg_size);
                            let inflight_labels = CallType::Server.to_labels::<R>(&req);
                            let _guard = RPC_METRICS.inflight[&inflight_labels].inc_guard(1);
                            let mut server_process_labels =
                                CallLatencyType::ServerProcess.to_labels::<R>(&req, &Ok(()));
                            let mut recv_send_labels =
                                CallLatencyType::ServerRecvSend.to_labels::<R>(&req, &Ok(()));

                            let process_time = ctx.now();
                            let res = self.handler.handle(ctx, req).await.context(R::METHOD);
                            server_process_labels.set_result(&res);
                            RPC_METRICS.latency[&server_process_labels]
                                .observe_latency(ctx.now() - process_time);

                            let res = frame::mux_send_proto(ctx, &mut stream.write, &res?).await;
                            recv_send_labels.set_result(&res);
                            RPC_METRICS.latency[&recv_send_labels]
                                .observe_latency(ctx.now() - recv_time);
                            let msg_size = res?;
                            RPC_METRICS.message_size[&resp_size_labels].observe(msg_size);
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
    pub(crate) fn add_server<R: Rpc>(
        mut self,
        handler: impl Handler<R> + 'a,
        rate: limiter::Rate,
    ) -> Self {
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
            rate,
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
