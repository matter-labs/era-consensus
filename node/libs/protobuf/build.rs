//! Generates rust code from protobufs.
fn main() {
    zksync_protobuf_build::Config {
        input_root: "src/proto".into(),
        proto_root: "zksync".into(),
        dependencies: vec![],
        protobuf_crate: "crate".parse().expect("protobuf_crate"),
    }
    .generate()
    .expect("generate(std)");

    zksync_protobuf_build::Config {
        input_root: "src/tests/proto".into(),
        proto_root: "zksync/protobuf/tests".into(),
        dependencies: vec![],
        protobuf_crate: "crate".parse().expect("protobuf_crate"),
    }
    .generate()
    .expect("generate(test)");

    zksync_protobuf_build::Config {
        input_root: "src/bin/conformance_test/proto".into(),
        proto_root: "zksync/protobuf/conformance_test".into(),
        dependencies: vec![],
        protobuf_crate: "::zksync_protobuf".parse().expect("protobuf_crate"),
    }
    .generate()
    .expect("generate(conformance)");
}
