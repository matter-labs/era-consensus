//! Generates rust code from protobufs.
fn main() {
    zksync_protobuf_build::Config {
        input_root: "src/proto".into(),
        proto_root: "zksync/network".into(),
        dependencies: vec![
            "::zksync_protobuf::proto".parse().unwrap(),
            "::zksync_consensus_roles::proto".parse().unwrap(),
        ],
        protobuf_crate: "::zksync_protobuf".parse().unwrap(),
        is_public: false,
    }
    .generate()
    .expect("generate()");

    zksync_protobuf_build::Config {
        input_root: "src/mux/tests/proto".into(),
        proto_root: "zksync/network/mux/tests".into(),
        dependencies: vec![],
        protobuf_crate: "::zksync_protobuf".parse().unwrap(),
        is_public: false,
    }
    .generate()
    .expect("generate()");
}
