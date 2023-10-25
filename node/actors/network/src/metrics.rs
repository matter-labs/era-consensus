//! General-purpose network metrics.

use crate::state::State;
use concurrency::{ctx, io, net};
use std::{
    net::SocketAddr,
    pin::Pin,
    sync::Weak,
    task::{ready, Context, Poll},
};
use vise::{
    Collector, Counter, EncodeLabelSet, EncodeLabelValue, Family, Gauge, GaugeGuard, Metrics, Unit,
};

/// Metered TCP stream.
#[pin_project::pin_project]
pub(crate) struct MeteredStream {
    #[pin]
    stream: net::tcp::Stream,
    _active: GaugeGuard,
}

impl MeteredStream {
    /// Opens a TCP connection to a remote host and returns a metered stream.
    pub(crate) async fn connect(
        ctx: &ctx::Ctx,
        addr: SocketAddr,
    ) -> ctx::OrCanceled<io::Result<Self>> {
        let io_result = net::tcp::connect(ctx, addr).await?;
        Ok(io_result.map(|stream| Self::new(stream, Direction::Outbound)))
    }

    /// Accepts an inbound connection and returns a metered stream.
    pub(crate) async fn listen(
        ctx: &ctx::Ctx,
        listener: &mut net::tcp::Listener,
    ) -> ctx::OrCanceled<io::Result<Self>> {
        let io_result = net::tcp::accept(ctx, listener).await?;
        Ok(io_result.map(|stream| Self::new(stream, Direction::Inbound)))
    }

    #[cfg(test)]
    pub(crate) async fn test_pipe(ctx: &ctx::Ctx) -> (Self, Self) {
        let (outbound_stream, inbound_stream) = net::tcp::testonly::pipe(ctx).await;
        let outbound_stream = Self::new(outbound_stream, Direction::Outbound);
        let inbound_stream = Self::new(inbound_stream, Direction::Inbound);
        (outbound_stream, inbound_stream)
    }

    fn new(stream: net::tcp::Stream, direction: Direction) -> Self {
        TCP_METRICS.established[&direction].inc();
        Self {
            stream,
            _active: TCP_METRICS.active[&direction].inc_guard(1),
        }
    }
}

impl io::AsyncRead for MeteredStream {
    #[inline(always)]
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.project();
        let before = buf.remaining();
        let res = this.stream.poll_read(cx, buf);
        let after = buf.remaining();
        TCP_METRICS.received.inc_by((before - after) as u64);
        res
    }
}

impl io::AsyncWrite for MeteredStream {
    #[inline(always)]
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context, buf: &[u8]) -> Poll<io::Result<usize>> {
        let this = self.project();
        let res = ready!(this.stream.poll_write(cx, buf))?;
        TCP_METRICS.sent.inc_by(res as u64);
        Poll::Ready(Ok(res))
    }

    #[inline(always)]
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
        self.project().stream.poll_flush(cx)
    }

    #[inline(always)]
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
        self.project().stream.poll_shutdown(cx)
    }
}

/// Direction of a TCP connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EncodeLabelSet, EncodeLabelValue)]
#[metrics(label = "direction", rename_all = "snake_case")]
enum Direction {
    /// Inbound connection.
    Inbound,
    /// Outbound connection.
    Outbound,
}

/// Metrics reported for TCP connections.
#[derive(Debug, Metrics)]
#[metrics(prefix = "network_tcp")]
struct TcpMetrics {
    /// Total bytes sent over all TCP connections.
    #[metrics(unit = Unit::Bytes)]
    sent: Counter,
    /// Total bytes received over all TCP connections.
    #[metrics(unit = Unit::Bytes)]
    received: Counter,
    /// TCP connections established since the process started.
    established: Family<Direction, Counter>,
    /// Number of currently active TCP connections.
    active: Family<Direction, Gauge>,
}

/// TCP metrics instance.
#[vise::register]
static TCP_METRICS: vise::Global<TcpMetrics> = vise::Global::new();

/// General-purpose network metrics exposed via a collector.
#[derive(Debug, Metrics)]
#[metrics(prefix = "network")]
pub(crate) struct NetworkGauges {
    /// Number of active inbound gossip connections.
    gossip_inbound_connections: Gauge<usize>,
    /// Number of active outbound gossip connections.
    gossip_outbound_connections: Gauge<usize>,
    /// Number of active inbound consensus connections.
    consensus_inbound_connections: Gauge<usize>,
    /// Number of active outbound consensus connections.
    consensus_outbound_connections: Gauge<usize>,
}

impl NetworkGauges {
    /// Registers a metrics collector for the specified state.
    pub(crate) fn register(state_ref: Weak<State>) {
        #[vise::register]
        static COLLECTOR: Collector<Option<NetworkGauges>> = Collector::new();

        let register_result = COLLECTOR.before_scrape(move || {
            state_ref.upgrade().map(|state| {
                let gauges = NetworkGauges::default();
                let len = state.gossip.inbound.subscribe().borrow().current().len();
                gauges.gossip_inbound_connections.set(len);
                let len = state.gossip.outbound.subscribe().borrow().current().len();
                gauges.gossip_outbound_connections.set(len);
                let len = state.consensus.inbound.subscribe().borrow().current().len();
                gauges.consensus_inbound_connections.set(len);
                let subscriber = state.consensus.outbound.subscribe();
                let len = subscriber.borrow().current().len();
                gauges.consensus_outbound_connections.set(len);
                gauges
            })
        });
        if register_result.is_err() {
            tracing::warn!("Failed registering network metrics collector: already registered");
        }
    }
}
