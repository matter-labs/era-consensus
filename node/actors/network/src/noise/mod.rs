//! Stream encrypted with noise protocol.
//! http://www.noiseprotocol.org/noise.html
pub(crate) mod bytes;
mod stream;

#[cfg(test)]
pub(super) mod testonly;
#[cfg(test)]
mod tests;

pub(crate) use stream::*;
