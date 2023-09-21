//! Context-aware async read/write operations.
use crate::ctx;
pub use io::{split, AsyncRead, AsyncWrite, Error, ErrorKind, ReadBuf, Result};
use tokio::{
    io,
    io::{AsyncReadExt as _, AsyncWriteExt as _},
};

/// Reads at most buf.len() bytes into `buf`, or returns an (ctx::Canceled or io::Error) error.
/// Returns the number of bytes read.
/// In case of error no bytes had been read from the stream.
pub async fn read<S: AsyncRead + Unpin>(
    ctx: &ctx::Ctx,
    stream: &mut S,
    buf: &mut [u8],
) -> ctx::OrCanceled<Result<usize>> {
    ctx.wait(stream.read(buf)).await
}

/// Reads buf.len() bytes into `buf`, or returns an (ctx::Canceled or io::Error) error.
/// In case of error some bytes might have been already read, but the content of `buf` is
/// unspecified.
pub async fn read_exact<S: AsyncRead + Unpin>(
    ctx: &ctx::Ctx,
    stream: &mut S,
    buf: &mut [u8],
) -> ctx::OrCanceled<Result<()>> {
    Ok(ctx.wait(stream.read_exact(buf)).await?.map(|_| ()))
}

/// Writes the whole `buf`, or returns an (ctx::Canceled or io::Error) error.
/// In case of error some bytes might have been already written.
pub async fn write_all<S: AsyncWrite + Unpin>(
    ctx: &ctx::Ctx,
    stream: &mut S,
    buf: &[u8],
) -> ctx::OrCanceled<Result<()>> {
    ctx.wait(stream.write_all(buf)).await
}

/// Flushes the write buffer.
pub async fn flush<S: AsyncWrite + Unpin>(
    ctx: &ctx::Ctx,
    stream: &mut S,
) -> ctx::OrCanceled<Result<()>> {
    ctx.wait(stream.flush()).await
}

/// Flushes the write buffer then shuts down the stream.
pub async fn shutdown<S: AsyncWrite + Unpin>(
    ctx: &ctx::Ctx,
    stream: &mut S,
) -> ctx::OrCanceled<Result<()>> {
    ctx.wait(stream.shutdown()).await
}
