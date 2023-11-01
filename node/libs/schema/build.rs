//! Generates rust code from the capnp schema files in the `capnp/` directory.
use std::{env, path::PathBuf};
use anyhow::Context as _;

fn main() -> anyhow::Result<()> {
    let input = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?).canonicalize()?;
    let output = PathBuf::from(std::env::var("OUT_DIR")?).canonicalize()?;
    protobuf::build::Config {
        input_root: input.join("proto"),
        dependencies: vec![&protobuf::proto::DESCRIPTOR],

        output_mod_path: output.join("proto/mod.rs"),
        output_descriptor_path: output.join("proto/desc.binpb"),
    
        protobuf_crate: "::protobuf".to_string(),
    }.generate().context("protobuf_build::Config::generate()")
}
