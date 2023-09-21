//! No-copy wrapper allowing to carry a `Copy` type into a closure or an `async` block.

use std::ops;

/// No-copy wrapper allowing to carry a `Copy` type into a closure or an `async` block.
#[derive(Debug)]
pub struct NoCopy<T>(T);

impl<T: Copy> NoCopy<T> {
    /// Converts this wrapper to the contained value.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T: Copy> From<T> for NoCopy<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}

impl<T: Copy> ops::Deref for NoCopy<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}
