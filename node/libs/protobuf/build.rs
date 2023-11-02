fn main() {
    protobuf_build::Config {
        input_path: "src/proto".into(),
        proto_path: "zksync".into(),
        dependencies: vec![],
        protobuf_crate: "crate".into(),
    }.generate().expect("generate(std)");

    protobuf_build::Config {
        input_path: "src/tests/proto".into(),
        proto_path: "zksync/protobuf/tests".into(),
        dependencies: vec![],
        protobuf_crate: "crate".into(),
    }.generate().expect("generate(test)");

    protobuf_build::Config {
        input_path: "src/bin/conformance_test/proto".into(),
        proto_path: "zksync/protobuf/conformance_test".into(),
        dependencies: vec![],
        protobuf_crate: "::protobuf".into(),
    }.generate().expect("generate(conformance)");
}
