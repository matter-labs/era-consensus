//! Generates rust code from protobufs.

fn main() {
    zksync_protobuf_build::Config {
        input_root: "src/bin/conformance_test/proto".into(),
        proto_root: "zksync/protobuf/conformance_test".into(),
        dependencies: vec![],
        protobuf_crate: "::zksync_protobuf".parse().expect("protobuf_crate"),
        is_public: false,
    }
    .generate()
    .expect("generate(conformance)");
}
