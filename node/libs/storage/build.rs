//! Generates rust code from protobufs.
fn main() {
    zksync_protobuf::build::Config {
        input_root: "src/proto".into(),
        proto_root: "zksync/storage".into(),
        dependencies: vec![(
            "::zksync_consensus_roles::proto".into(),
            &zksync_consensus_roles::proto::DESCRIPTOR,
        )],
        protobuf_crate: "::zksync_protobuf".into(),
    }
    .generate()
    .expect("generate()");
}
