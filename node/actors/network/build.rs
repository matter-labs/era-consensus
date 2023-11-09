//! Generates rust code from protobufs.
fn main() {
    zksync_protobuf::build::Config {
        input_root: "src/proto".into(),
        proto_root: "zksync/network".into(),
        dependencies: vec![
            (
                "::zksync_protobuf::proto".parse().unwrap(),
                &zksync_protobuf::proto::DESCRIPTOR,
            ),
            (
                "::zksync_consensus_roles::proto".parse().unwrap(),
                &zksync_consensus_roles::proto::DESCRIPTOR,
            ),
        ],
        protobuf_crate: "::zksync_protobuf".parse().unwrap(),
    }
    .generate()
    .expect("generate()");

    zksync_protobuf::build::Config {
        input_root: "src/mux/tests/proto".into(),
        proto_root: "zksync/network/mux/tests".into(),
        dependencies: vec![],
        protobuf_crate: "::zksync_protobuf".parse().unwrap(),
    }
    .generate()
    .expect("generate()");
}
