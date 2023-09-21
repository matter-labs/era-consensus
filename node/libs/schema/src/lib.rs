//! Code generated from protobuf schema files and
//! utilities for serialization.

mod proto_fmt;
mod std_conv;
pub mod testonly;

pub use proto_fmt::*;

#[cfg(test)]
mod tests;

#[allow(warnings)]
pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/proto/mod.rs"));
    static DESCRIPTOR_POOL: once_cell::sync::Lazy<prost_reflect::DescriptorPool> =
        once_cell::sync::Lazy::new(|| {
            prost_reflect::DescriptorPool::decode(
                include_bytes!(concat!(env!("OUT_DIR"), "/proto/descriptor.bin")).as_ref(),
            )
            .unwrap()
        });
}
