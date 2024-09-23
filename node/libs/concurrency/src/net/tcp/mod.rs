//! Context-aware utilities for `tokio::net::tcp`.
//! Note that `accept()` and `connect()` disable Nagle
//! algorithm (so that the transmission latency is more
//! predictable), so the caller is expected to apply
//! user space buffering.
use crate::ctx;
pub use listener_addr::*;
use tokio::io;

mod listener_addr;
pub mod testonly;

/// TCP stream.
pub type Stream = tokio::net::TcpStream;
/// TCP listener.
pub type Listener = tokio::net::TcpListener;

/// Accepts an INBOUND listener connection.
pub async fn accept(ctx: &ctx::Ctx, this: &mut Listener) -> ctx::OrCanceled<io::Result<Stream>> {
    ctx.wait(async {
        let stream = this.accept().await?.0;
        stream.set_nodelay(true)?;
        Ok(stream)
    })
    .await
}

/// Opens a TCP connection to a remote host.
pub async fn connect(
    ctx: &ctx::Ctx,
    addr: std::net::SocketAddr,
) -> ctx::OrCanceled<io::Result<Stream>> {
    ctx.wait(async {
        let stream = tokio::net::TcpStream::connect(addr).await?;
        stream.set_nodelay(true)?;
        Ok(stream)
    })
    .await
}
