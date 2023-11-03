fn main() {
    protobuf_build::Config {
        input_root: "src/proto".into(),
        proto_root: "zksync".into(),
        dependencies: vec![],
        protobuf_crate: "crate".into(),
    }.generate().expect("generate(std)");

    protobuf_build::Config {
        input_root: "src/tests/proto".into(),
        proto_root: "zksync/protobuf/tests".into(),
        dependencies: vec![],
        protobuf_crate: "crate".into(),
    }.generate().expect("generate(test)");

    protobuf_build::Config {
        input_root: "src/bin/conformance_test/proto".into(),
        proto_root: "zksync/protobuf/conformance_test".into(),
        dependencies: vec![],
        protobuf_crate: "::protobuf".into(),
    }.generate().expect("generate(conformance)");
}
