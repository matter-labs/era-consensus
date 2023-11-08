//! Generates rust code from protobufs.
fn main() {
    zksync_protobuf::build::Config {
        input_root: "src/config/proto".into(),
        proto_root: "zksync/executor/config".into(),
        dependencies: vec![],
        protobuf_crate: "::zksync_protobuf".into(),
    }
    .generate()
    .expect("generate(config)");
}
