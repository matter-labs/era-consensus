//! Generates rust code from protobufs.
fn main() {
    zksync_protobuf::build::Config {
        input_root: "proto".into(),
        dependencies: vec![(
            "::zksync_protobuf::proto".into(),
            &zksync_protobuf::proto::DESCRIPTOR,
        )],
        protobuf_crate: "::zksync_protobuf".into(),
    }
    .generate()
    .expect("generate()");
}
