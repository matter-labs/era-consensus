//! No-copy wrapper allowing to carry a `Copy` type into a closure or an `async` block.

/// No-copy wrapper allowing to carry a `Copy` type into a closure or an `async` block.
#[derive(Clone, Debug)]
pub struct NoCopy<T>(pub T);

impl<T> NoCopy<T> {
    /// Extracts the wrapped value.
    pub fn into(self) -> T { self.0 }
}

impl<T> std::ops::Deref for NoCopy<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}
