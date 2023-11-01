//! Generates rust code from the capnp schema files in the `capnp/` directory.
use std::{path::PathBuf, env};
use anyhow::Context as _;

fn main() -> anyhow::Result<()> {
    let input = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?).canonicalize()?;
    let output = PathBuf::from(std::env::var("OUT_DIR")?).canonicalize()?;
    protobuf_build::Config {
        proto_path: input.join("proto"),
        proto_package: "".to_string(),
        dependencies: vec![],

        output_mod_path: output.join("proto/mod.rs"),
        output_descriptor_path: output.join("proto/desc.binpb"),
    
        protobuf_crate: "crate".to_string(),
    }.generate().context("protobuf_build::Config::generate()")
}
