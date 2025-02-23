//! Internal state of a reusable stream.
use std::sync::Arc;

use zksync_concurrency::{ctx, ctx::channel, limiter, oneshot, scope, sync};

use super::{
    Config, FrameKind, Header, ReadStream, RunError, Stream, StreamId, StreamKind, WriteStream,
};
use crate::noise::bytes;

/// Read frame allocation permit.
#[derive(Debug)]
pub(super) struct ReadPermit {
    /// Permit for read_frame_count limit.
    pub(super) _count: sync::OwnedSemaphorePermit,
    /// Permit for read_buffer_size limit.
    pub(super) _size: sync::OwnedSemaphorePermit,
}

/// Mux protocol frame.
#[derive(Debug)]
pub(super) struct Frame {
    /// Frame header.
    pub(super) header: Header,
    /// Frame data. Present iff `header.frame_kind() == FrameKind::DATA`.
    /// If present, it is always nonempty.
    pub(super) data: Option<bytes::Buffer>,
    /// Frame permit. Present iff the frame is for reading.
    /// NOTE: we rely here on the order of dropping the fields:
    /// data should be dropped BEFORE permit. It doesn't matter much, but
    /// if it wasn't the case, in a very pathological situation we might
    /// exceed for a short moment the total allowed buffer size/frame count.
    pub(super) _permit: Option<ReadPermit>,
}

/// Commands send to a task which owns the write half of the stream.
#[derive(Debug)]
pub(super) enum WriteCommand {
    Frame(Frame),
    Flush,
}

/// Internal: channel over which the reserved stream is retrieved.
type Reservation = oneshot::Sender<Stream>;

/// Transient stream which can be opened later.
pub(crate) struct ReservedStream(oneshot::Sender<Reservation>);

impl ReservedStream {
    /// Opens the reserved stream.
    /// Returns `sync::Disconnected` if the reserved transient
    /// stream has been closed in the meantime.
    pub(crate) async fn open(
        self,
        ctx: &ctx::Ctx,
    ) -> ctx::OrCanceled<Result<Stream, sync::Disconnected>> {
        let (send, recv) = oneshot::channel();
        if self.0.send(send).is_err() {
            // Transport has been closed.
            return Ok(Err(sync::Disconnected));
        }
        recv.recv_or_disconnected(ctx).await
    }
}

/// Queue of established transient streams for a single capability.
/// Streams are being established on demand: One side needs to actually call
/// `queue.pop()` before the OPEN message is sent to the peer.
pub(crate) struct StreamQueue {
    pub(super) max_streams: u32,
    limiter: limiter::Limiter,
    send: channel::Sender<ReservedStream>,
    recv: sync::Mutex<channel::Receiver<ReservedStream>>,
}

impl StreamQueue {
    /// Constructs a new StreamQueue with the specified number of reusable streams.
    /// During multiplexer handshake, peers exchange information about
    /// how many reusable streams they support per capability.
    pub(crate) fn new(ctx: &ctx::Ctx, max_streams: u32, rate: limiter::Rate) -> Arc<Self> {
        let (send, recv) = channel::bounded(1);
        Arc::new(Self {
            max_streams,
            limiter: limiter::Limiter::new(ctx, rate),
            send,
            recv: sync::Mutex::new(recv),
        })
    }

    /// Reserves a transient stream from the queue to open later.
    /// Reservations can be placed in a common capacity pool.
    pub(crate) async fn reserve(&self, ctx: &ctx::Ctx) -> ctx::OrCanceled<ReservedStream> {
        let mut recv = sync::lock(ctx, &self.recv).await?.into_async();
        recv.recv(ctx).await
    }

    /// Opens a transient stream from the queue.
    #[allow(dead_code)]
    pub(crate) async fn open(&self, ctx: &ctx::Ctx) -> ctx::OrCanceled<Stream> {
        loop {
            // It may happen that the popped stream has been immediately disconnected
            // but that is a transient error.
            if let Ok(stream) = self.reserve(ctx).await?.open(ctx).await? {
                return Ok(stream);
            }
        }
    }

    /// Internal: pushes a ReservedStream to the queue and returns
    /// a corresponding Reservation.
    async fn push(&self, ctx: &ctx::Ctx) -> ctx::OrCanceled<Reservation> {
        loop {
            let (send, recv) = oneshot::channel();
            self.send.send(ctx, ReservedStream(send)).await?;
            if let Ok(reservation) = recv.recv_or_disconnected(ctx).await? {
                return Ok(reservation);
            }
        }
    }
}

/// State of read half of a reusable stream.
#[derive(Debug)]
pub(super) struct ReadReusableStream {
    /// None means that no frame is cached.
    /// This field is only used when accepting a new inbound connection.
    /// There is no one to process the frame until the stream is accepted.
    pub(crate) cache: Option<Frame>,
    /// Channel that provides frames for reading.
    /// Always check `cache` first before awaiting this channel.
    pub(crate) recv: channel::UnboundedReceiver<Frame>,
    /// Whether a CLOSE frame has been received.
    pub(crate) close_received: bool,
}

/// State of write half of a reusable stream.
#[derive(Debug)]
pub(super) struct WriteReusableStream {
    /// Runner that this reusable stream belongs to.
    pub(crate) cfg: Arc<Config>,
    /// ID of the reusable stream.
    pub(crate) stream_id: StreamId,
    /// Kind of the reusable stream.
    pub(crate) stream_kind: StreamKind,
    /// Cached data frame to send.
    pub(crate) buffer: bytes::Buffer,
    /// Channel used to schedule the frames for sending.
    pub(super) write_send: channel::Sender<WriteCommand>,
    /// A notification scheduling the stream flush.
    /// We use it instead of the `send` channel
    /// to delay the flushing in case there are some
    /// frames scheduled for sending, and to deduplicate
    /// flushing requests coming from multiple streams.
    pub(super) flush: Arc<sync::Notify>,
}

impl ReadReusableStream {
    pub(super) fn new(recv: channel::UnboundedReceiver<Frame>) -> Self {
        Self {
            cache: None,
            recv,
            close_received: false,
        }
    }

    pub(super) async fn recv_open(&mut self, ctx: &ctx::Ctx) -> ctx::OrCanceled<()> {
        self.cache.take();
        self.close_received = false;
        while self.recv.recv(ctx).await?.header.frame_kind() != FrameKind::OPEN {}
        Ok(())
    }
}

impl WriteReusableStream {
    pub(super) fn new(
        cfg: Arc<Config>,
        stream_id: StreamId,
        stream_kind: StreamKind,
        write_send: channel::Sender<WriteCommand>,
        flush: Arc<sync::Notify>,
    ) -> Self {
        Self {
            stream_id,
            stream_kind,
            buffer: bytes::Buffer::new(cfg.write_frame_size as usize),
            write_send,
            flush,
            cfg,
        }
    }

    pub(super) async fn send_data(&mut self, ctx: &ctx::Ctx) -> Result<(), RunError> {
        if self.buffer.len() == 0 {
            return Ok(());
        }
        let slot = self
            .write_send
            .reserve_or_disconnected(ctx)
            .await?
            .map_err(|_| RunError::Closed)?;
        let header = Header::new(FrameKind::DATA, self.stream_kind, self.stream_id);
        let frame = Frame {
            header,
            data: Some(std::mem::replace(
                &mut self.buffer,
                bytes::Buffer::new(self.cfg.write_frame_size as usize),
            )),
            _permit: None,
        };
        slot.send(WriteCommand::Frame(frame));
        Ok(())
    }

    /// Send the CLOSE frame.
    pub(super) async fn send_close(&mut self, ctx: &ctx::Ctx) -> Result<(), RunError> {
        self.send_data(ctx).await?;
        let header = Header::new(FrameKind::CLOSE, self.stream_kind, self.stream_id);
        let frame = Frame {
            header,
            data: None,
            _permit: None,
        };
        self.write_send
            .send(ctx, WriteCommand::Frame(frame))
            .await?;
        self.flush.notify_one();
        Ok(())
    }

    pub(super) async fn send_open(&mut self, ctx: &ctx::Ctx) -> Result<(), RunError> {
        let header = Header::new(FrameKind::OPEN, self.stream_kind, self.stream_id);
        let frame = Frame {
            header,
            data: None,
            _permit: None,
        };
        self.write_send
            .send(ctx, WriteCommand::Frame(frame))
            .await?;
        self.flush.notify_one();
        Ok(())
    }
}

/// Shared state of the Reusable stream.
pub(super) struct ReusableStream {
    /// Read half of the stream.
    pub(super) read: ReadReusableStream,
    /// Write half of the stream.
    pub(super) write: WriteReusableStream,
    /// A queue through which fresh transient streams for the given capability will be requested.
    pub(super) stream_queue: Arc<StreamQueue>,
}

impl ReusableStream {
    /// Runs the reusable stream protocol.
    /// In a loop:
    /// * waits for the user to request a transient stream.
    /// * performs a 3-way handshake using OPEN frames.
    /// * passes the established transient stream to the user, who can send/recv DATA frames
    /// * once user drops the transient stream, a CLOSE frame is sent.
    pub(super) async fn run(self, ctx: &ctx::Ctx) -> Result<(), RunError> {
        scope::run!(ctx, |ctx, s| async {
            // Since the locks are immediately dropped, the read / write streams are immediately available.
            let (_, mut read_receiver) = sync::ExclusiveLock::new(self.read);
            let (_, mut write_receiver) = sync::ExclusiveLock::new(self.write);

            loop {
                let recv_open_task = s.spawn(async {
                    let mut read = read_receiver.wait(ctx).await?;
                    read.recv_open(ctx).await?;
                    Ok(read)
                });
                let mut write = write_receiver.wait(ctx).await?;
                write.send_close(ctx).await?;

                let _open_permit = self.stream_queue.limiter.acquire(ctx, 1).await?;
                let (read, reservation) = match write.stream_kind {
                    StreamKind::ACCEPT => {
                        let read = recv_open_task.join(ctx).await?;
                        let reservation = self.stream_queue.push(ctx).await?;
                        write.send_open(ctx).await?;
                        (read, reservation)
                    }
                    StreamKind::CONNECT => {
                        let reservation = self.stream_queue.push(ctx).await?;
                        write.send_open(ctx).await?;
                        let read = recv_open_task.join(ctx).await?;
                        (read, reservation)
                    }
                    _ => unreachable!("bad StreamKind"),
                };

                let (read_lock, new_read_receiver) = sync::ExclusiveLock::new(read);
                read_receiver = new_read_receiver;
                let (write_lock, new_write_receiver) = sync::ExclusiveLock::new(write);
                write_receiver = new_write_receiver;
                // Sending may fail because the requester is not interested in the stream any more.
                // In this case we just close the transient stream immediately.
                let _ = reservation.send(Stream {
                    read: ReadStream(read_lock),
                    write: WriteStream(write_lock),
                });
            }
        })
        .await
    }
}
