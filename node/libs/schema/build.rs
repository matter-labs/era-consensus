//! Generates rust code from the capnp schema files in the `capnp/` directory.
use std::{env, path::PathBuf};
use anyhow::Context as _;

fn main() -> anyhow::Result<()> {
    let input = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?).canonicalize()?;
    let output = PathBuf::from(std::env::var("OUT_DIR")?).canonicalize()?;
    protobuf::build::Config {
        input_path: input.join("proto"),
        proto_path: "zksync/schema".into(),
        proto_package: "zksync.schema".into(),
        dependencies: vec![("::protobuf::proto",&protobuf::proto::DESCRIPTOR)],

        output_mod_path: output.join("proto.gen.rs"),
        output_descriptor_path: output.join("proto.gen.binpb"),
    
        protobuf_crate: "::protobuf".into(),
    }.generate().context("generate()")
}
