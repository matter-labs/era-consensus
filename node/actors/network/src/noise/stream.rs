//! `tokio::io` stream using Noise encryption.
use super::bytes;
use crate::metrics::MeteredStream;
use std::{
    pin::Pin,
    task::{ready, Context, Poll},
};
use zksync_concurrency::{
    ctx, io,
    io::{AsyncRead as _, AsyncWrite as _},
};
use zksync_consensus_crypto::{keccak256::Keccak256, ByteFmt};

/// Fixed noise configuration. Nodes need to use the same noise
/// configuration to be able to connect to each other.
fn params() -> snow::params::NoiseParams {
    snow::params::NoiseParams {
        name: "zksync-bft".to_string(),
        base: snow::params::BaseChoice::Noise,
        handshake: snow::params::HandshakeChoice {
            // We use NN noise variant, in which both sides use just ephemeral keys.
            // Authentication which uses static keys is performed after the noise handshake.
            pattern: snow::params::HandshakePattern::NN,
            modifiers: snow::params::HandshakeModifierList { list: vec![] },
        },
        // We use Curve25519 Diffie-Hellman protocol to establish a common secret.
        dh: snow::params::DHChoice::Curve25519,
        // We use ChaChaPoly symmetric encryption for communication.
        cipher: snow::params::CipherChoice::ChaChaPoly,
        // We use SHA256 for hashing used in the noise protocol.
        hash: snow::params::HashChoice::SHA256,
    }
}

// Constants from the Noise spec.

/// Maximal size of the encrypted frame that Noise may output.
// TODO: we can use smaller buffers, in case we need a lot of connections.
const MAX_TRANSPORT_MSG_LEN: usize = 65535;

/// Size of data appended/prepended by the noise protocol to the payload of a Noise frame.
const AUTHDATA_LEN: usize = 16;
/// Maximal size of the payload that can be included in a noise frame.
const MAX_PAYLOAD_LEN: usize = MAX_TRANSPORT_MSG_LEN - AUTHDATA_LEN;

/// The Noise encrypted frame has variable length, so we wrap it in a simple framing format:
/// <frame len:u16> ++ <frame data:[u8;len]>.
///
/// Length of the frame len field.
const LENGTH_FIELD_LEN: usize = std::mem::size_of::<u16>();

/// Max size of the whole frame (length field + data).
const MAX_FRAME_LEN: usize = MAX_TRANSPORT_MSG_LEN + LENGTH_FIELD_LEN;

/// Buffers used by the stream.
struct Buffer {
    /// Unencrypted payload buffer.
    payload: bytes::Buffer,
    /// Encrypted frame buffer (with the length field as a prefix).
    frame: bytes::Buffer,
}

impl Default for Buffer {
    fn default() -> Self {
        Self {
            payload: bytes::Buffer::new(MAX_PAYLOAD_LEN),
            frame: bytes::Buffer::new(MAX_FRAME_LEN),
        }
    }
}

/// Encrypted stream.
/// It implements tokio::io::AsyncRead/AsyncWrite.
#[pin_project::pin_project(project = StreamProject)]
pub(crate) struct Stream<S = MeteredStream> {
    /// Hash of the handshake messages.
    /// Uniquely identifies the noise session.
    id: Keccak256,
    /// Underlying TCP stream.
    #[pin]
    inner: S,
    /// Noise protocol state, used to encrypt/decrypt frames.
    noise: snow::TransportState,
    /// Buffers used for the read half of the stream.
    read_buf: Box<Buffer>,
    /// Buffers used for the write half of the stream.
    write_buf: Box<Buffer>,
}

impl<S> std::ops::Deref for Stream<S> {
    type Target = S;
    fn deref(&self) -> &S {
        &self.inner
    }
}

impl<S> Stream<S>
where
    S: io::AsyncRead + io::AsyncWrite + Unpin,
{
    /// Performs a server-side noise handshake and returns the encrypted stream.
    pub(crate) async fn server_handshake(ctx: &ctx::Ctx, stream: S) -> anyhow::Result<Self> {
        Self::handshake(ctx, stream, snow::Builder::new(params()).build_responder()?).await
    }

    /// Performs a client-side noise handshake and returns the encrypted stream.
    pub(crate) async fn client_handshake(ctx: &ctx::Ctx, stream: S) -> anyhow::Result<Self> {
        Self::handshake(ctx, stream, snow::Builder::new(params()).build_initiator()?).await
    }

    /// Performs the noise handshake given the HandshakeState.
    async fn handshake(
        ctx: &ctx::Ctx,
        mut stream: S,
        mut hs: snow::HandshakeState,
    ) -> anyhow::Result<Self> {
        let mut buf = vec![0; 65536];
        let mut payload = vec![];
        loop {
            if hs.is_handshake_finished() {
                return Ok(Self {
                    id: ByteFmt::decode(hs.get_handshake_hash()).unwrap(),
                    inner: stream,
                    noise: hs.into_transport_mode()?,
                    read_buf: Box::default(),
                    write_buf: Box::default(),
                });
            }
            if hs.is_my_turn() {
                let n = hs.write_message(&payload, &mut buf)?;
                // TODO(gprusak): writing/reading length field and the frame content could be
                // done in a single syscall.
                io::write_all(ctx, &mut stream, &u16::to_le_bytes(n as u16)).await??;
                io::write_all(ctx, &mut stream, &buf[..n]).await??;
                io::flush(ctx, &mut stream).await??;
            } else {
                let mut msg_size = [0u8, 2];
                io::read_exact(ctx, &mut stream, &mut msg_size).await??;
                let n = u16::from_le_bytes(msg_size) as usize;
                io::read_exact(ctx, &mut stream, &mut buf[..n]).await??;
                hs.read_message(&buf[..n], &mut payload)?;
            }
        }
    }

    /// Returns the noise session id.
    /// See `Stream::id`.
    pub(crate) fn id(&self) -> Keccak256 {
        self.id
    }

    /// Wait until a frame is fully loaded.
    /// Returns the size of the frame.
    /// Returns None in case EOF is reached before the frame is loaded.
    fn poll_read_frame(
        this: &mut StreamProject<'_, S>,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<Option<usize>>> {
        // Fetch frame until complete.
        loop {
            if this.read_buf.frame.len() >= LENGTH_FIELD_LEN {
                let n = u16::from_le_bytes(this.read_buf.frame.prefix()) as usize;
                if this.read_buf.frame.len() >= LENGTH_FIELD_LEN + n {
                    return Poll::Ready(Ok(Some(n)));
                }
            }
            let n = {
                let mut frame = io::ReadBuf::new(this.read_buf.frame.as_mut_capacity());
                ready!(Pin::new(&mut this.inner).poll_read(cx, &mut frame))?;
                frame.filled().len()
            };
            // propagate EOF.
            if n == 0 {
                return Poll::Ready(Ok(None));
            }
            this.read_buf.frame.extend(n);
        }
    }

    /// Wait until payload is nonempty.
    fn poll_read_payload(
        this: &mut StreamProject<'_, S>,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        if this.read_buf.payload.len() > 0 {
            return Poll::Ready(Ok(()));
        }
        let Some(n) = ready!(Self::poll_read_frame(this, cx))? else {
            return Poll::Ready(Ok(()));
        };
        this.read_buf.payload.reset();
        let m = this
            .noise
            .read_message(
                &this.read_buf.frame.as_slice()[LENGTH_FIELD_LEN..LENGTH_FIELD_LEN + n],
                this.read_buf.payload.as_mut_capacity(),
            )
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        this.read_buf.frame.take(LENGTH_FIELD_LEN + n);
        this.read_buf.frame.shift();
        this.read_buf.payload.extend(m);
        Poll::Ready(Ok(()))
    }
}

impl<S> io::AsyncRead for Stream<S>
where
    S: io::AsyncRead + io::AsyncWrite + Unpin,
{
    /// From tokio::io::AsyncRead:
    /// * The amount of data read can be determined by the increase
    ///   in the length of the slice returned by ReadBuf::filled.
    /// * If the difference is 0, EOF has been reached.
    /// From std::io::Read:
    /// * If error was returned, no bytes were read.
    ///
    /// Hence it is responsibility of the caller to make sure that
    /// `buf.remaining()>0`.
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let mut this = self.project();
        ready!(Self::poll_read_payload(&mut this, cx))?;
        let n = std::cmp::min(buf.remaining(), this.read_buf.payload.len());
        buf.put_slice(&this.read_buf.payload.as_slice()[..n]);
        this.read_buf.payload.take(n);
        Poll::Ready(Ok(()))
    }
}

impl<S> Stream<S>
where
    S: io::AsyncRead + io::AsyncWrite + Unpin,
{
    /// poll_flush_frame will either flush this.write_buf.frame, or return an error.
    fn poll_flush_frame(
        this: &mut StreamProject<'_, S>,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        while this.write_buf.frame.len() > 0 {
            let n =
                ready!(Pin::new(&mut this.inner).poll_write(cx, this.write_buf.frame.as_slice()))?;
            if n == 0 {
                return Poll::Ready(Err(io::ErrorKind::WriteZero.into()));
            }
            this.write_buf.frame.take(n);
        }
        Poll::Ready(Ok(()))
    }

    /// poll_flush_payload will either flush this.write_buf.payload, or return an error.
    fn poll_flush_payload(
        this: &mut StreamProject<'_, S>,
        cx: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        if this.write_buf.payload.len() == 0 {
            return Poll::Ready(Ok(()));
        }
        ready!(Self::poll_flush_frame(this, cx))?;
        this.write_buf.frame.reset();
        // write_message() may fail if self.noise is in a broken state (for example: all nonces
        // have been used up). It should never fail due to input being too large, or output buffer
        // being too small.
        let n = this
            .noise
            .write_message(
                this.write_buf.payload.as_slice(),
                &mut this.write_buf.frame.as_mut_capacity()[LENGTH_FIELD_LEN..],
            )
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        this.write_buf.frame.set_prefix((n as u16).to_le_bytes());
        this.write_buf.frame.extend(LENGTH_FIELD_LEN + n);
        this.write_buf.payload.take(this.write_buf.payload.len());
        this.write_buf.payload.reset();
        Poll::Ready(Ok(()))
    }
}

impl<S> io::AsyncWrite for Stream<S>
where
    S: io::AsyncRead + io::AsyncWrite + Unpin,
{
    /// from futures::io::AsyncWrite:
    /// * poll_write must try to make progress by flushing if needed to become writable
    /// from std::io::Write:
    /// * call to write represents at most one attempt to write to any wrapped object.
    /// * 0 TYPICALLY means that that the underlying object is no longer able to accept bytes OR
    ///   that buf.len()==0.
    /// * if an error is returned then no bytes in the buffer were written to this writer.
    ///
    /// Stream::poll_write will write >0 bytes (to the buffers) or return an error.
    /// Returns Ok(0) iff buf.len()==0.
    /// It may call self.inner.poll_write multiple times to free enough memory in buffers to put
    /// some bytes into them.
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }
        let mut this = self.project();
        if this.write_buf.payload.capacity() == 0 {
            ready!(Self::poll_flush_payload(&mut this, cx))?;
        }
        let n = this.write_buf.payload.push(buf);
        debug_assert!(n > 0);
        Poll::Ready(Ok(n))
    }

    /// attempt to flush the object, ensuring that any buffered data reach their destination.
    /// poll_flush will either successfully flush all the data, or return an error.
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let mut this = self.project();
        ready!(Self::poll_flush_payload(&mut this, cx))?;
        ready!(Self::poll_flush_frame(&mut this, cx))?;
        ready!(this.inner.poll_flush(cx))?;
        Poll::Ready(Ok(()))
    }

    /// from `tokio::io::AsyncWrite::poll_shutdown`:
    /// * attempts to shut down, returning success when I/O connection has COMPLETELY shut down.
    /// * it is the hook to implement the graceful shutdown logic.
    /// * invocation of shutdown implies an invocation of flush.
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let mut this = self.project();
        ready!(Self::poll_flush_payload(&mut this, cx))?;
        ready!(Self::poll_flush_frame(&mut this, cx))?;
        ready!(this.inner.poll_shutdown(cx))?;
        Poll::Ready(Ok(()))
    }
}
