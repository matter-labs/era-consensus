//! Code generated from protobuf schema files and
//! utilities for serialization.

pub mod proto;
mod proto_fmt;
mod std_conv;
pub mod testonly;

pub use proto_fmt::*;
pub use zksync_protobuf_build as build;

#[cfg(test)]
mod tests;
