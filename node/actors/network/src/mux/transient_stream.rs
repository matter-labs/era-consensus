//! Transient stream representation.
//!
//! It doesn't implement `io::AsyncRead` or `io::AsyncWrite`,
//! because these have semantics which is hard to grasp if
//! the object is not the sole owner of the underlying stream.
//!
//! The API of the transient stream is not final.
//! It might get adjusted later for better buffer management
//! or convenience of use.
use super::{FrameKind, ReadReusableStream, WriteReusableStream};
use crate::noise::bytes;
use zksync_concurrency::{ctx, sync};

/// Read half of the transient stream.
#[derive(Debug)]
pub(crate) struct ReadStream(pub(super) sync::ExclusiveLock<ReadReusableStream>);

/// Write half of the transient stream.
#[derive(Debug)]
pub(crate) struct WriteStream(pub(super) sync::ExclusiveLock<WriteReusableStream>);

impl ReadStream {
    /// Reads until buf is full, or end of stream is reached.
    pub(crate) async fn read_exact(
        &mut self,
        ctx: &ctx::Ctx,
        buf: &mut bytes::Buffer,
    ) -> anyhow::Result<()> {
        loop {
            if self.0.close_received {
                return Ok(());
            }
            let mut frame = match self.0.cache.take() {
                Some(frame) => frame,
                None => match self.0.recv.recv_or_disconnected(ctx).await? {
                    Ok(frame) => frame,
                    // Transport termination is equivalent to EOS.
                    Err(sync::Disconnected) => return Ok(()),
                },
            };
            match frame.header.frame_kind() {
                FrameKind::OPEN => {
                    tracing::warn!("unexpected OPEN frame");
                }
                FrameKind::CLOSE => {
                    self.0.close_received = true;
                }
                FrameKind::DATA => {
                    let data = frame.data.as_mut().unwrap();
                    data.take(buf.push(data.as_slice()));
                    if data.len() > 0 {
                        self.0.cache = Some(frame);
                    }
                    if buf.capacity() == 0 {
                        return Ok(());
                    }
                }
                _ => unreachable!("Bad FrameKind"),
            }
        }
    }
}

impl WriteStream {
    /// Writes `buf` to the stream.
    /// On success, all the data has been written to the stream.
    /// On error, part of the data may have been written to the stream.
    pub(crate) async fn write_all(&mut self, ctx: &ctx::Ctx, buf: &[u8]) -> anyhow::Result<()> {
        let mut offset = 0;
        while offset < buf.len() {
            if self.0.buffer.capacity() == 0 {
                self.0.send_data(ctx).await?;
            }
            offset += self.0.buffer.push(&buf[offset..]);
        }
        Ok(())
    }

    /// Notifies the transport stream to flush the stream.
    // Remove once we have a use case in prod code for this.
    #[allow(dead_code)]
    pub(crate) async fn flush(&mut self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        self.0.send_data(ctx).await?;
        self.0.flush.notify_one();
        Ok(())
    }
}

/// A bidirectional transient stream.
/// Consists of a read-half and write-half.
/// Drop the write half to notify the peer that EOS has been reached.
/// Drop the whole stream to close it entirely and allow unlock the stream quota.
#[derive(Debug)]
pub(crate) struct Stream {
    /// Read half of the transient stream.
    pub(crate) read: ReadStream,
    /// Write half of the transient stream.
    pub(crate) write: WriteStream,
}
