//! Context-aware utilities for `tokio::net::tcp`.
//! Note that `accept()` and `connect()` disable Nagle
//! algorithm (so that the transmission latency is more
//! predictable), so the caller is expected to apply
//! user space buffering.
use crate::{ctx, metrics};
pub use listener_addr::*;
use once_cell::sync::Lazy;
use std::{
    pin::Pin,
    task::{ready, Context, Poll},
};
use tokio::io;

mod listener_addr;
pub mod testonly;

static BYTES_SENT: Lazy<prometheus::IntCounter> = Lazy::new(|| {
    prometheus::register_int_counter!(
        "concurrency_net_tcp__bytes_sent",
        "bytes sent over TCP connections"
    )
    .unwrap()
});
static BYTES_RECV: Lazy<prometheus::IntCounter> = Lazy::new(|| {
    prometheus::register_int_counter!(
        "concurrency_net_tcp__bytes_recv",
        "bytes received over TCP connections"
    )
    .unwrap()
});
static ESTABLISHED: Lazy<prometheus::IntCounterVec> = Lazy::new(|| {
    prometheus::register_int_counter_vec!(
        "concurrency_net_tcp__established",
        "TCP connections established since the process started",
        &["direction"]
    )
    .unwrap()
});
static ACTIVE: Lazy<prometheus::IntGaugeVec> = Lazy::new(|| {
    prometheus::register_int_gauge_vec!(
        "concurrency_net_tcp__active",
        "currently active TCP connections",
        &["direction"]
    )
    .unwrap()
});

/// TCP stream.
#[pin_project::pin_project]
pub struct Stream {
    #[pin]
    stream: tokio::net::TcpStream,
    _active: metrics::GaugeGuard<prometheus::core::AtomicI64>,
}

/// TCP listener.
pub type Listener = tokio::net::TcpListener;

/// Accepts an INBOUND listener connection.
pub async fn accept(ctx: &ctx::Ctx, this: &mut Listener) -> ctx::OrCanceled<io::Result<Stream>> {
    const INBOUND: &[&str] = &["inbound"];
    Ok(ctx.wait(this.accept()).await?.map(|stream| {
        ESTABLISHED.with_label_values(INBOUND).inc();
        // We are the only owner of the correctly opened
        // socket at this point so `set_nodelay` should
        // always succeed.
        stream.0.set_nodelay(true).unwrap();
        Stream {
            stream: stream.0,
            _active: ACTIVE.with_label_values(INBOUND).into(),
        }
    }))
}

/// Opens a TCP connection to a remote host.
pub async fn connect(
    ctx: &ctx::Ctx,
    addr: std::net::SocketAddr,
) -> ctx::OrCanceled<io::Result<Stream>> {
    const OUTBOUND: &[&str] = &["outbound"];
    Ok(ctx
        .wait(tokio::net::TcpStream::connect(addr))
        .await?
        .map(|stream| {
            ESTABLISHED.with_label_values(OUTBOUND).inc();
            // We are the only owner of the correctly opened
            // socket at this point so `set_nodelay` should
            // always succeed.
            stream.set_nodelay(true).unwrap();
            Stream {
                stream,
                _active: ACTIVE.with_label_values(OUTBOUND).into(),
            }
        }))
}

impl io::AsyncRead for Stream {
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
        BYTES_RECV.inc_by((before - after) as u64);
        res
    }
}

impl io::AsyncWrite for Stream {
    #[inline(always)]
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context, buf: &[u8]) -> Poll<io::Result<usize>> {
        let this = self.project();
        let res = ready!(this.stream.poll_write(cx, buf))?;
        BYTES_SENT.inc_by(res as u64);
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
