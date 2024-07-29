//! Stream multiplexer. Allows to communicate over multiple independent transient substreams,
//! over a single transport stream. Single multiplexer may define multiple internal endpoints (aka
//! "capabilities"). When establishing a new transient substream (via accept/connect methods)
//! you need to specify a capability: if peer A calls `mux.connect(ctx,x)` then peer B needs to
//! call `mux.accept(ctx,x)` to accept A's substream (x is the capability and it has
//! to match on both sides of the connection).
//!
//! It currently doesn't prevent head-of-line blocking,
//! which means that if data on one transient substream is not consumed fast enough,
//! another substream can get blocked due to insufficient buffer space.
//!
//! Call `mux::Mux::new(cfg)` to obtain a new multiplexer.
//! Then call `mux.run(ctx,transport)` in a concurrent task to exchange messages on the `transport`.
//! Call `mux.accept(ctx,cap)` to accept a new inbound transient substream on capability `cap`.
//! Call `mux.connect(cfg,cap)` to open a new outbound transient substream on capability `cap`.
//!
//! See `mux/tests.rs` for usage example.
//!
//!
//! == Mux protocol specification ==
//!
//! Multiplexer emulates a collection of transient streams using a single transport stream.
//! We define the following types of streams:
//! * transport stream - the low level `io::AsyncRead + io::AsyncWrite` stream which is owned
//!   by the multiplexer. `Runner::run()` method is responsible for reading/writing frames in
//!   multiplexer format (defined below) to this stream and dispatching the data to the transient streams.
//! * reusable stream - an internal notion of the multiplexer protocol (see below)
//! * transient stream - an independent bidirectional stream of data used by the user of the
//!   multiplexer. You can start a new outbound transient stream by calling mux.connect,
//!   and you can accept a new inbound transient stream by calling mux.accept.
//!
//! Multiplexer defines a fixed space of reusable stream IDs:
//! * 2^13 inbound streams (where me = server, peer = client)
//! * 2^13 outbound streams (where me = client, peer = server)
//!
//! The exact number of reusable streams is determined by the `accept` and `connect` fields of the
//! config. For a given capability `x` we will have `min(me.cfg.accept[x].max_streams,peer.cfg.connect[x].max_streams)`
//! reusable inbound streams (analogically for outbound streams). Stream IDs are assigned to
//! capabilities in an ascending order of capability ID. For example, if
//! * capability 0 has 4 reusable streams
//! * capability 1 has 3 reusable streams
//! * capability 2 has 5 reusable streams
//! then
//! * streams 0,1,2,3 belong to capability 0.
//! * streams 4,5,6 belong to capability 1.
//! * streams 7,8,9,10,11 belong to capability 2.
//! NOTE that the 2^13 limit applies to the TOTAL number of reusable streams across all
//! capabilities.
//!
//! Communication on each reusable stream is independent, except for the shared capacity of the
//! read buffers (TODO(gprusak): make the read buffers also independent).
//!
//! There are 3 kinds of frames exchanged over the transport stream:
//! * OPEN frame: used to establish a new transient stream
//! * DATA frame: used to send the actual data over the transient stream
//! * CLOSE frame: used to indicate end of the transient stream (i.e. no more DATA frames will be
//!   sent).
//!
//! Multipexer protocol:
//! 1. peer A and B exchange their multiplexer configs.
//! 2. based on both configs, A and B determine how many stream IDs will be used and to which
//!   capabilities they will belong.
//! 3. For every stream IDs in use execute Reusable Stream protocol concurrently.
//!
//! Reusable stream protocol (for stream ID = x, with capability = c, where A is outbound peer and
//! B is inbound peer):
//! 1. A sends OPEN(x) frame to B
//! 2. B received OPEN(x) frame from A, and then sends OPEN(x) frame to A
//! 3. A receives OPEN(x) frame from B, transient stream has been established.
//! 4. In parallel:
//! * A sends a sequence of DATA(x) frames to B, ending with a CLOSE(x) frame
//! * B sends a sequence of DATA(x) frames to A, ending with a CLOSE(x) frame.
//! * A and B read the DATA/CLOSE frames sent by the peer.
//! 5. transient stream has been closed. Go back to 1.
//!
//! Note that both A and B can delay sending OPEN frame,
//! therefore stopping the peer from sending any DATA frames.
//! This can be used to implement a rate limiting strategy that
//! both sides of the connection can enforce.
use crate::{frame, noise::bytes};
use anyhow::Context as _;
use std::{collections::BTreeMap, sync::Arc};
use zksync_concurrency::{ctx, ctx::channel, error::Wrap as _, io, scope, sync};

mod config;
mod handshake;
mod header;
mod reusable_stream;
#[cfg(test)]
mod tests;
mod transient_stream;

pub(crate) use config::*;
use handshake::Handshake;
use header::{FrameKind, Header, StreamId, StreamKind};
pub(crate) use reusable_stream::*;
pub(crate) use transient_stream::*;

/// Multiplexer.
/// Supports accepting new inbound substreams
/// and starting new outbound substreams.
pub(crate) struct Mux {
    /// Transport config.
    pub(crate) cfg: Arc<Config>,
    /// StreamQueues of "accept" streams per capability.
    pub(crate) accept: BTreeMap<CapabilityId, Arc<StreamQueue>>,
    /// StreamQueues of "connect" streams per capability.
    pub(crate) connect: BTreeMap<CapabilityId, Arc<StreamQueue>>,
}

fn saturating_sum(iter: impl Iterator<Item = u32>) -> u32 {
    iter.fold(0, |x, v| x.saturating_add(v))
}

impl Mux {
    /// Generates a handshake message for the multiplexer.
    fn handshake(&self) -> Handshake {
        Handshake {
            accept_max_streams: self
                .accept
                .iter()
                .map(|(id, q)| (*id, q.max_streams))
                .collect(),
            connect_max_streams: self
                .connect
                .iter()
                .map(|(id, q)| (*id, q.max_streams))
                .collect(),
        }
    }

    /// Verifies the config correctness.
    pub(crate) fn verify(&self) -> anyhow::Result<()> {
        self.cfg.verify().context("cfg")?;
        if saturating_sum(self.accept.values().map(|cfg| cfg.max_streams)) > MAX_STREAM_COUNT {
            anyhow::bail!("sum of accept_inflight > {MAX_STREAM_COUNT}");
        }
        if saturating_sum(self.connect.values().map(|cfg| cfg.max_streams)) > MAX_STREAM_COUNT {
            anyhow::bail!("sum of connect_inflight > {MAX_STREAM_COUNT}");
        }
        Ok(())
    }
}

/// Error returned by `Mux::run()`.
#[derive(Debug, thiserror::Error)]
#[allow(clippy::missing_docs_in_private_items)]
pub(crate) enum RunError {
    #[error("config: {0:#}")]
    Config(anyhow::Error),
    #[error(transparent)]
    Canceled(#[from] ctx::Canceled),
    #[error("connection closed")]
    Closed,
    #[error("protocol: {0:#}")]
    Protocol(anyhow::Error),
    #[error(transparent)]
    IO(#[from] io::Error),
}

impl From<RunError> for ctx::Error {
    fn from(err: RunError) -> Self {
        match err {
            RunError::Canceled(err) => Self::Canceled(err),
            err => Self::Internal(err.into()),
        }
    }
}

impl Mux {
    async fn spawn_streams<'env>(
        &self,
        ctx: &'env ctx::Ctx,
        scope: &scope::Scope<'env, RunError>,
        stream_kind: StreamKind,
        handshake: &Handshake,
        write_send: &channel::Sender<WriteCommand>,
        flush: &Arc<sync::Notify>,
    ) -> Vec<channel::UnboundedSender<Frame>> {
        let mut streams = vec![];
        let (queues, peer) = match stream_kind {
            StreamKind::ACCEPT => (&self.accept, &handshake.connect_max_streams),
            StreamKind::CONNECT => (&self.connect, &handshake.accept_max_streams),
            _ => unreachable!("bad StreamKind"),
        };
        for (cap, queue) in queues {
            let max_streams = std::cmp::min(queue.max_streams, *peer.get(cap).unwrap_or(&0));
            for _ in 0..max_streams {
                let (read_send, read_recv) = channel::unbounded();
                let stream_id = StreamId::new(streams.len() as u16);
                streams.push(read_send);
                let stream = ReusableStream {
                    read: ReadReusableStream::new(read_recv),
                    write: WriteReusableStream::new(
                        self.cfg.clone(),
                        stream_id,
                        stream_kind,
                        write_send.clone(),
                        flush.clone(),
                    ),
                    stream_queue: queue.clone(),
                };
                scope.spawn_bg(stream.run(ctx));
            }
        }
        streams
    }

    async fn process_inbound_frames(
        &self,
        ctx: &ctx::Ctx,
        mut read: impl io::AsyncRead + Send + Unpin,
        accept_streams: Vec<channel::UnboundedSender<Frame>>,
        connect_streams: Vec<channel::UnboundedSender<Frame>>,
    ) -> Result<(), RunError> {
        let count_sem = Arc::new(sync::Semaphore::new(self.cfg.read_frame_count as usize));
        let size_sem = Arc::new(sync::Semaphore::new(self.cfg.read_buffer_size as usize));
        loop {
            let mut header = [0u8, 2];
            io::read_exact(ctx, &mut read, &mut header).await??;
            let header = Header::from(header);
            // If the frame was sent by the inbound end of the stream, then it should be
            // handled by the outbound end, and vice versa.
            let streams = match header.stream_kind() {
                StreamKind::ACCEPT => &connect_streams,
                StreamKind::CONNECT => &accept_streams,
                _ => unreachable!("bad StreamKind"),
            };
            let stream = streams
                .get(header.stream_id().0 as usize)
                .with_context(|| format!("bad stream id {:?}", header.stream_id()))
                .map_err(RunError::Protocol)?;

            match header.frame_kind() {
                FrameKind::OPEN | FrameKind::CLOSE => {
                    let permit = Some(ReadPermit {
                        _count: sync::acquire_many_owned(ctx, count_sem.clone(), 1).await?,
                        _size: size_sem.clone().try_acquire_many_owned(0).unwrap(),
                    });
                    stream.send(Frame {
                        header,
                        data: None,
                        _permit: permit,
                    });
                }
                FrameKind::DATA => {
                    let mut length = [0u8, 2];
                    io::read_exact(ctx, &mut read, &mut length).await??;
                    let mut length = u16::from_le_bytes(length) as usize;
                    // Split into frames of `read_frame_size` size.
                    while length > 0 {
                        let size = std::cmp::min(length, self.cfg.read_frame_size as usize);
                        let permit = Some(ReadPermit {
                            _count: sync::acquire_many_owned(ctx, count_sem.clone(), 1).await?,
                            _size: sync::acquire_many_owned(ctx, size_sem.clone(), size as u32)
                                .await?,
                        });
                        let mut data = bytes::Buffer::new(size);
                        io::read_exact(ctx, &mut read, data.as_mut_capacity()).await??;
                        data.extend(size);
                        stream.send(Frame {
                            header,
                            data: Some(data),
                            _permit: permit,
                        });
                        length -= size;
                    }
                }
                _ => unreachable!("bad FrameKind"),
            }
        }
    }

    /// Executes the Multiplexer protocol.
    /// First the multiplexer configs are exchanged.
    /// Then for every agreed reusable stream a task managing that stream is spawned (both inbound
    /// and outbound).
    pub(crate) async fn run<S: io::AsyncRead + io::AsyncWrite + Send>(
        self,
        ctx: &ctx::Ctx,
        transport: S,
    ) -> Result<(), RunError> {
        self.verify().map_err(RunError::Config)?;
        let (mut read, mut write) = io::split(transport);
        let res = scope::run!(ctx, |ctx, s| async {
            s.spawn(async {
                let h = self.handshake();
                frame::send_proto(ctx, &mut write, &h)
                    .await
                    .wrap("send_proto()")
            });
            frame::recv_proto(ctx, &mut read, handshake::MAX_FRAME)
                .await
                .wrap("recv_proto()")
        })
        .await;

        let handshake = res.map_err(|err| match err {
            ctx::Error::Canceled(err) => RunError::Canceled(err),
            ctx::Error::Internal(err) => RunError::Protocol(err),
        })?;

        let (write_send, write_recv) = channel::bounded(1);
        let flush = Arc::new(sync::Notify::new());
        let res = scope::run!(ctx, |ctx, s| async {
            let accept_streams = self
                .spawn_streams(ctx, s, StreamKind::ACCEPT, &handshake, &write_send, &flush)
                .await;
            let connect_streams = self
                .spawn_streams(ctx, s, StreamKind::CONNECT, &handshake, &write_send, &flush)
                .await;

            s.spawn_bg::<()>(async {
                let mut write = write;
                let mut write_recv = write_recv;
                loop {
                    match write_recv.recv(ctx).await? {
                        WriteCommand::Flush => io::flush(ctx, &mut write).await??,
                        WriteCommand::Frame(frame) => {
                            io::write_all(ctx, &mut write, &frame.header.raw()).await??;
                            if let Some(data) = frame.data {
                                let length = (data.len() as u16).to_le_bytes();
                                io::write_all(ctx, &mut write, &length).await??;
                                io::write_all(ctx, &mut write, data.as_slice()).await??;
                            }
                        }
                    }
                }
            });
            s.spawn_bg::<()>(async {
                loop {
                    sync::notified(ctx, &flush).await?;
                    // TODO(gprusak): first call this.write.send().reserve()
                    // and then clear this.flush before sending a flush.
                    write_send.send(ctx, WriteCommand::Flush).await?;
                }
            });

            self.process_inbound_frames(ctx, read, accept_streams, connect_streams)
                .await
        })
        .await;

        match res {
            Ok(()) => unreachable!(),
            Err(RunError::IO(err)) => match err.kind() {
                io::ErrorKind::UnexpectedEof
                | io::ErrorKind::ConnectionReset
                | io::ErrorKind::BrokenPipe => Err(RunError::Closed),
                _ => Err(RunError::IO(err)),
            },
            err => err,
        }
    }
}
