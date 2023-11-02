//! Generates rust code from the capnp schema files in the `capnp/` directory.
use std::{path::PathBuf, env};
use anyhow::Context as _;

fn main() -> anyhow::Result<()> {
    let input = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?).canonicalize()?;
    let output = PathBuf::from(std::env::var("OUT_DIR")?).canonicalize()?;
    protobuf_build::Config {
        input_path: input.join("src/proto"),
        proto_path: "zksync".into(),
        proto_package: "zksync".into(),
        dependencies: vec![],

        output_mod_path: output.join("src/proto.gen.rs"),
        output_descriptor_path: output.join("src/proto.gen.binpb"),
    
        protobuf_crate: "crate".into(),
    }.generate().context("generate(std)")?;

    protobuf_build::Config {
        input_path: input.join("src/tests/proto"),
        proto_path: "zksync/protobuf/tests".into(),
        proto_package: "zksync.protobuf.tests".into(),
        dependencies: vec![],

        output_mod_path: output.join("src/tests/proto.gen.rs"),
        output_descriptor_path: output.join("src/tests/proto.gen.binpb"),
    
        protobuf_crate: "crate".into(),
    }.generate().context("generate(test)")?;

    protobuf_build::Config {
        input_path: input.join("src/bin/conformance_test/proto"),
        proto_path: "zksync/protobuf/conformance_test".into(),
        proto_package: "zksync.protobuf.conformance_test".into(),
        dependencies: vec![],

        output_mod_path: output.join("src/bin/conformance_test/proto.gen.rs"),
        output_descriptor_path: output.join("src/bin/conformance_test/proto.gen.binpb"),
    
        protobuf_crate: "::protobuf".into(),
    }.generate().context("generate(conformance)")?;

    Ok(())
}
