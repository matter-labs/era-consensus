use super::StreamId;
use concurrency::sync;

/// Maximal frame size.
pub(crate) const MAX_FRAME_SIZE: u64 = u16::MAX as u64;
/// Maximal number of streams.
pub(crate) const MAX_STREAM_COUNT: u32 = (StreamId::MASK + 1) as u32;
/// Maximal number of read frames.
pub(crate) const MAX_READ_FRAME_COUNT: u64 = sync::Semaphore::MAX_PERMITS as u64;
/// Maximal total size of read buffers.
pub(crate) const MAX_READ_BUFFER_SIZE: u64 = sync::Semaphore::MAX_PERMITS as u64;

/// Capability ID
pub(crate) type CapabilityId = u64;

/// Multiplexer config.
#[derive(Debug, Clone)]
pub(crate) struct Config {
    /// Maximal size (in bytes) of the data frame passed to the stream.
    /// Has to be lower than 2^16.
    /// If a larger frame is received from the peer, it will be split into multiple frames.
    /// It is important, because a frame of size N bytes can be fetched and processed,
    /// N bytes need to be reserved from read_buffer_size pool.
    /// lower read_frame size => better consumption granularity & higher communication overhead.
    pub(crate) read_frame_size: u64,
    /// Limit on the total unconsumed read data (in bytes).
    /// It is an upper bound on the sum of sizes of read frames.
    /// It prevents a DoS attack in case the peer sends too much data.
    pub(crate) read_buffer_size: u64,
    /// Limit on the total number of unconsumed frames.
    /// It prevents a DoS attack in case the peer sends too many frames.
    /// It is complementary to read_buffer_size, which doesn't take into consideration
    /// the per-frame overhead.
    /// TODO(gprusak): it should be possible to combine read_buffer_size and read_frame_count
    /// into a single parameter by determining their relative cost.
    /// It is implementation dependent however (i.e. how efficiently we operate on the frames)
    /// so let it be an explicit parameter for now.
    pub(crate) read_frame_count: u64,

    /// Limit on the size of a single frame written to the transport stream.
    /// Has to be lower than 2^16.
    /// It limits the amount of data that is buffered for writing,
    /// which is useful in case we want to cancel early the `Stream::write_all()` method.
    /// (mux protocol requires to write full frames, but we can close the
    /// transient stream after every frame).
    pub(crate) write_frame_size: u64,
}

impl Config {
    /// Verifies correctness of the config.
    pub(crate) fn verify(&self) -> anyhow::Result<()> {
        if self.write_frame_size > MAX_FRAME_SIZE {
            anyhow::bail!("write_frame_size > {MAX_FRAME_SIZE}");
        }
        if self.read_buffer_size > MAX_READ_BUFFER_SIZE {
            anyhow::bail!("read_buffer_size > {MAX_READ_BUFFER_SIZE}");
        }
        if self.read_frame_count > MAX_READ_FRAME_COUNT {
            anyhow::bail!("read_frame_count > {MAX_READ_FRAME_COUNT}");
        }
        Ok(())
    }
}
