//! Simple frame encoding format (length ++ value) for protobuf messages,
//! since protobuf messages do not have delimiters.
use crate::{mux, noise::bytes};
use anyhow::Context as _;
use zksync_concurrency::{ctx, io};

/// Reads a raw frame of bytes from the stream and interprets it as proto.
/// A `frame : [u8]` is encoded as `L ++ frame`, where `L` is
/// a little endian encoding of `frame.len() as u32`.
/// Returns the decoded proto and the size of the received message in bytes.
pub(crate) async fn mux_recv_proto<T: zksync_protobuf::ProtoFmt>(
    ctx: &ctx::Ctx,
    stream: &mut mux::ReadStream,
    max_size: usize,
) -> anyhow::Result<(T, usize)> {
    let mut msg_size = bytes::Buffer::new(4);
    stream.read_exact(ctx, &mut msg_size).await?;
    if msg_size.capacity() != 0 {
        anyhow::bail!("end of stream");
    }
    let msg_size = u32::from_le_bytes(msg_size.prefix()) as usize;
    if msg_size > max_size {
        anyhow::bail!("message too large: max = {}B, got {msg_size}B", max_size);
    }
    let mut msg = bytes::Buffer::new(msg_size);
    stream.read_exact(ctx, &mut msg).await?;
    if msg.len() < msg_size {
        anyhow::bail!("end of stream");
    }
    let msg = zksync_protobuf::decode(msg.as_slice())?;
    Ok((msg, msg_size))
}

/// Sends a proto serialized to a raw frame of bytes to the stream.
/// It doesn't flush the stream.
/// Returns the size of the sent proto in bytes.
pub(crate) async fn mux_send_proto<T: zksync_protobuf::ProtoFmt>(
    ctx: &ctx::Ctx,
    stream: &mut mux::WriteStream,
    msg: &T,
) -> anyhow::Result<usize> {
    let msg = zksync_protobuf::encode(msg);
    stream
        .write_all(ctx, &u32::to_le_bytes(msg.len().try_into()?))
        .await?;
    stream.write_all(ctx, &msg).await?;
    Ok(msg.len())
}

/// Reads a raw frame of bytes from the stream and interprets it as proto.
/// A `frame : [u8]` is encoded as `L ++ frame`, where `L` is
/// a little endian encoding of `frame.len() as u32`.
pub(crate) async fn recv_proto<T: zksync_protobuf::ProtoFmt, S: io::AsyncRead + Unpin>(
    ctx: &ctx::Ctx,
    stream: &mut S,
    max_size: usize,
) -> ctx::Result<T> {
    let mut msg_size = [0u8; 4];
    io::read_exact(ctx, stream, &mut msg_size)
        .await?
        .context("read_exact(len)")?;
    let msg_size = u32::from_le_bytes(msg_size);
    if msg_size as usize > max_size {
        return Err(
            anyhow::format_err!("message too large: max = {max_size}, got {msg_size}",).into(),
        );
    }
    let mut msg = vec![0u8; msg_size as usize];
    io::read_exact(ctx, stream, &mut msg[..])
        .await?
        .context("read_exact(msg)")?;
    Ok(zksync_protobuf::decode(&msg).context("decode()")?)
}

/// Sends a proto serialized to a raw frame of bytes to the stream.
pub(crate) async fn send_proto<T: zksync_protobuf::ProtoFmt, S: io::AsyncWrite + Unpin>(
    ctx: &ctx::Ctx,
    stream: &mut S,
    msg: &T,
) -> ctx::Result<()> {
    let msg = zksync_protobuf::encode(msg);
    io::write_all(
        ctx,
        stream,
        &u32::to_le_bytes(msg.len().try_into().context("msg.len()")?),
    )
    .await?
    .context("write(len)")?;
    io::write_all(ctx, stream, &msg)
        .await?
        .context("write(msg)")?;
    io::flush(ctx, stream).await?.context("flush")?;
    Ok(())
}
