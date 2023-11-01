//! Code generated from protobuf schema files and
//! utilities for serialization.

mod proto_fmt;
mod std_conv;
pub mod testonly;

pub use proto_fmt::*;
pub use protobuf_build as build;

#[cfg(test)]
mod tests;

#[allow(warnings)]
pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/proto/mod.rs"));
}
