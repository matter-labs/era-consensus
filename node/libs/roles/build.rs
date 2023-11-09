//! Generates rust code from protobufs.
fn main() {
    zksync_protobuf::build::Config {
        input_root: "src/proto".into(),
        proto_root: "zksync/roles".into(),
        dependencies: vec![(
            "::zksync_protobuf::proto".parse().expect("dependency_path"),
            &zksync_protobuf::proto::DESCRIPTOR,
        )],
        protobuf_crate: "::zksync_protobuf".parse().expect("protobuf_crate"),
    }
    .generate()
    .expect("generate()");
}
