//! Context-aware utilities for `tokio::net::tcp`.
//! Note that `accept()` and `connect()` disable Nagle
//! algorithm (so that the transmission latency is more
//! predictable), so the caller is expected to apply
//! user space buffering.
use crate::{
    ctx,
    metrics::{self, Direction},
};
pub use listener_addr::*;
use std::{
    pin::Pin,
    task::{ready, Context, Poll},
};
use tokio::io;

mod listener_addr;
pub mod testonly;

/// TCP stream.
#[pin_project::pin_project]
pub struct Stream {
    #[pin]
    stream: tokio::net::TcpStream,
    _active: metrics::GaugeGuard,
}

/// TCP listener.
pub type Listener = tokio::net::TcpListener;

/// Accepts an INBOUND listener connection.
pub async fn accept(ctx: &ctx::Ctx, this: &mut Listener) -> ctx::OrCanceled<io::Result<Stream>> {
    Ok(ctx.wait(this.accept()).await?.map(|stream| {
        metrics::TCP_METRICS.established_connections[&Direction::Inbound].inc();

        // We are the only owner of the correctly opened
        // socket at this point so `set_nodelay` should
        // always succeed.
        stream.0.set_nodelay(true).unwrap();
        Stream {
            stream: stream.0,
            _active: metrics::TCP_METRICS.active_connections[&Direction::Inbound]
                .clone()
                .into(),
        }
    }))
}

/// Opens a TCP connection to a remote host.
pub async fn connect(
    ctx: &ctx::Ctx,
    addr: std::net::SocketAddr,
) -> ctx::OrCanceled<io::Result<Stream>> {
    Ok(ctx
        .wait(tokio::net::TcpStream::connect(addr))
        .await?
        .map(|stream| {
            metrics::TCP_METRICS.established_connections[&Direction::Outbound].inc();
            // We are the only owner of the correctly opened
            // socket at this point so `set_nodelay` should
            // always succeed.
            stream.set_nodelay(true).unwrap();
            Stream {
                stream,
                _active: metrics::TCP_METRICS.active_connections[&Direction::Outbound]
                    .clone()
                    .into(),
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
        metrics::TCP_METRICS
            .received
            .inc_by((before - after) as u64);
        res
    }
}

impl io::AsyncWrite for Stream {
    #[inline(always)]
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context, buf: &[u8]) -> Poll<io::Result<usize>> {
        let this = self.project();
        let res = ready!(this.stream.poll_write(cx, buf))?;
        metrics::TCP_METRICS.sent.inc_by(res as u64);
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
