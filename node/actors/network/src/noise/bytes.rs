//! Custom buffer used by the noise Stream.

use std::fmt;

/// Buffer of size `SIZE`.
/// Content of the buffer is stored in `inner[begin..end]`.
/// This is NOT a cyclical buffer: invariant `begin <= end`
/// is always valid.
pub(crate) struct Buffer {
    /// Underlying bytes array.
    inner: Box<[u8]>,
    /// Begin of the buffer content range.
    begin: usize,
    /// End of the buffer content range.
    end: usize,
}

impl fmt::Debug for Buffer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Buffer")
            .field("len", &self.len())
            .finish()
    }
}

impl Buffer {
    /// Constructs a new buffer with the given capacity.
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            inner: vec![0; capacity].into_boxed_slice(),
            begin: 0,
            end: 0,
        }
    }

    /// Length of the buffer content.
    pub(crate) fn len(&self) -> usize {
        self.end - self.begin
    }

    /// Remaining capacity of the buffer.
    pub(crate) fn capacity(&self) -> usize {
        self.inner.len() - self.end
    }

    /// Appends bytes from `buf` to the buffer until it is full.
    /// Returns the number of bytes appended.
    pub(crate) fn push(&mut self, buf: &[u8]) -> usize {
        let n = std::cmp::min(self.capacity(), buf.len());
        self.inner[self.end..self.end + n].copy_from_slice(&buf[..n]);
        self.end += n;
        n
    }

    /// Mutable reference to the remaining capacity in the buffer.
    /// Once you modify the returned slice, use `extend()` to
    /// mark how many bytes you have added.
    pub(crate) fn as_mut_capacity(&mut self) -> &mut [u8] {
        &mut self.inner[self.end..]
    }

    /// Marks `n` bytes from the remaining capacity
    /// as buffer content. This method should be called
    /// after the call to `as_mut_capacity()`.
    pub(crate) fn extend(&mut self, n: usize) {
        debug_assert!(self.end + n <= self.inner.len());
        self.end += n;
    }

    /// Content of the buffer.
    pub(crate) fn as_slice(&self) -> &[u8] {
        &self.inner[self.begin..self.end]
    }

    /// Consumes `n` bytes from the buffer.
    pub(crate) fn take(&mut self, n: usize) {
        debug_assert!(self.begin + n <= self.end);
        self.begin += n;
    }

    /// Peeks initial `N` bytes from the buffer.
    /// It does NOT consume any bytes from the buffer.
    pub(crate) fn prefix<const N: usize>(&self) -> [u8; N] {
        debug_assert!(self.begin + N <= self.end);
        self.inner[self.begin..self.begin + N].try_into().unwrap()
    }

    /// Sets the initial `N` bytes of the remaining capacity to `prefix`.
    /// Panics if `capacity() < N`.
    /// Use `extend()` to mark those bytes as buffer content.
    pub(crate) fn set_prefix<const N: usize>(&mut self, prefix: [u8; N]) {
        *<&mut [u8; N]>::try_from(&mut self.inner[self.end..self.end + N]).unwrap() = prefix;
    }

    /// Shifts the content of the buffer to the start
    /// of the underlying array. Since `Buffer` is not a cyclical
    /// buffer, prefix of the underlying array becomes unused after
    /// the bytes are consumed. Call `shift()` to regain the lost
    /// capacity.
    pub(crate) fn shift(&mut self) {
        self.inner.copy_within(self.begin..self.end, 0);
        self.end -= self.begin;
        self.begin = 0;
    }

    /// Clears the buffer.
    pub(crate) fn reset(&mut self) {
        self.begin = 0;
        self.end = 0;
    }
}
