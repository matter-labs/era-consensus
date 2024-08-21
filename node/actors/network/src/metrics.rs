//! General-purpose network metrics.
use crate::Network;
use anyhow::Context as _;
use std::{
    fmt,
    net::SocketAddr,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Weak,
    },
    task::{ready, Context, Poll},
    time::SystemTime,
};
use vise::{
    Collector, Counter, EncodeLabelSet, EncodeLabelValue, Family, Gauge, GaugeGuard, Metrics, Unit,
};
use zksync_concurrency::{ctx, io, net};

/// Metered TCP stream.
#[pin_project::pin_project]
pub(crate) struct MeteredStream {
    #[pin]
    stream: net::tcp::Stream,
    /// Collects values to be shown on the Debug http page
    stats: Arc<MeteredStreamStats>,
    _active: GaugeGuard,
}

impl MeteredStream {
    /// Opens a TCP connection to a remote host and returns a metered stream.
    pub(crate) async fn connect(ctx: &ctx::Ctx, addr: SocketAddr) -> ctx::Result<Self> {
        let stream = net::tcp::connect(ctx, addr).await?.context("connect()")?;
        Ok(Self::new(stream, Direction::Outbound)?)
    }

    /// Accepts an inbound connection and returns a metered stream.
    pub(crate) async fn accept(
        ctx: &ctx::Ctx,
        listener: &mut net::tcp::Listener,
    ) -> ctx::Result<Self> {
        let stream = net::tcp::accept(ctx, listener).await?.context("accept()")?;
        Ok(Self::new(stream, Direction::Inbound)?)
    }

    #[cfg(test)]
    pub(crate) async fn test_pipe(ctx: &ctx::Ctx) -> (Self, Self) {
        let (outbound_stream, inbound_stream) = net::tcp::testonly::pipe(ctx).await;
        let outbound_stream = Self::new(outbound_stream, Direction::Outbound).unwrap();
        let inbound_stream = Self::new(inbound_stream, Direction::Inbound).unwrap();
        (outbound_stream, inbound_stream)
    }

    fn new(stream: net::tcp::Stream, direction: Direction) -> anyhow::Result<Self> {
        TCP_METRICS.established[&direction].inc();
        let addr = stream.peer_addr().context("peer_addr()")?;
        Ok(Self {
            stream,
            stats: Arc::new(MeteredStreamStats::new(addr)),
            _active: TCP_METRICS.active[&direction].inc_guard(1),
        })
    }

    /// Returns a reference to the the Stream values for inspection
    pub(crate) fn stats(&self) -> Arc<MeteredStreamStats> {
        self.stats.clone()
    }
}

impl std::ops::Deref for MeteredStream {
    type Target = net::tcp::Stream;
    fn deref(&self) -> &Self::Target {
        &self.stream
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
        let amount = (before - buf.remaining()) as u64;
        TCP_METRICS.received.inc_by(amount);
        this.stats.read(amount);
        res
    }
}

impl io::AsyncWrite for MeteredStream {
    #[inline(always)]
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context, buf: &[u8]) -> Poll<io::Result<usize>> {
        let this = self.project();
        let res = ready!(this.stream.poll_write(cx, buf))?;
        TCP_METRICS.sent.inc_by(res as u64);
        this.stats.wrote(res as u64);
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

/// `build_version` label.
#[derive(Clone, Debug, PartialEq, Eq, Hash, EncodeLabelSet, EncodeLabelValue)]
#[metrics(label = "build_version")]
pub(crate) struct BuildVersion(String);

// For the isolated metric label to work, you should implement `Display` for it:
impl fmt::Display for BuildVersion {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(fmt)
    }
}

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
    /// Number of peers (both inbound and outbound) with the given `build_version`.
    /// Label "" corresponds to peers with build_version not set.
    /// WARNING: with the current implementation we do not bound
    /// the allowed set of BuildVersion values. This is not a threat
    /// to the node itself, but the prometheus scraper might be.
    gossip_peers_by_build_version: Family<BuildVersion, Gauge<usize>>,
}

impl NetworkGauges {
    /// Registers a metrics collector for the specified state.
    pub(crate) fn register(state_ref: Weak<Network>) {
        #[vise::register]
        static COLLECTOR: Collector<Option<NetworkGauges>> = Collector::new();

        let register_result = COLLECTOR.before_scrape(move || {
            state_ref.upgrade().map(|state| {
                let gauges = NetworkGauges::default();
                let inbound = state.gossip.inbound.current();
                gauges.gossip_inbound_connections.set(inbound.len());
                for conn in inbound.values() {
                    let v = BuildVersion(
                        conn.build_version
                            .as_ref()
                            .map(|v| v.to_string())
                            .unwrap_or_default(),
                    );
                    gauges.gossip_peers_by_build_version[&v].inc_by(1);
                }
                let outbound = state.gossip.outbound.current();
                gauges.gossip_outbound_connections.set(outbound.len());
                for conn in outbound.values() {
                    let v = BuildVersion(
                        conn.build_version
                            .as_ref()
                            .map(|v| v.to_string())
                            .unwrap_or_default(),
                    );
                    gauges.gossip_peers_by_build_version[&v].inc_by(1);
                }
                if let Some(consensus_state) = &state.consensus {
                    let len = consensus_state.inbound.current().len();
                    gauges.consensus_inbound_connections.set(len);
                    let len = consensus_state.outbound.current().len();
                    gauges.consensus_outbound_connections.set(len);
                }
                gauges
            })
        });
        if register_result.is_err() {
            tracing::warn!("Failed registering network metrics collector: already registered");
        }
    }
}

/// Metrics reported for TCP connections.
#[derive(Debug)]
pub struct MeteredStreamStats {
    /// Total bytes sent over the Stream.
    pub sent: AtomicU64,
    /// Total bytes received over the Stream.
    pub received: AtomicU64,
    /// System time since the connection started.
    pub established: SystemTime,
    /// Ip Address and port of current connection.
    pub peer_addr: SocketAddr,
}

impl MeteredStreamStats {
    fn new(peer_addr: SocketAddr) -> Self {
        Self {
            sent: 0.into(),
            received: 0.into(),
            established: SystemTime::now(),
            peer_addr,
        }
    }

    fn read(&self, amount: u64) {
        self.received.fetch_add(amount, Ordering::Relaxed);
    }

    fn wrote(&self, amount: u64) {
        self.sent.fetch_add(amount, Ordering::Relaxed);
    }
}
