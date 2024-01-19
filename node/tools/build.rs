//! Generates rust code from protobufs.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    zksync_protobuf_build::Config {
        input_root: "src/proto".into(),
        proto_root: "zksync/tools".into(),
        dependencies: vec![],
        protobuf_crate: "::zksync_protobuf".parse().unwrap(),
        is_public: false,
    }
    .generate()
    .unwrap();

    tonic_build::compile_protos("src/proto_test/node.proto")?;
    Ok(())
}
