//! Generates rust code from the capnp schema files in the `capnp/` directory.
fn main() {
    protobuf::build::Config {
        input_path: "proto".into(),
        proto_path: "zksync/schema".into(),
        dependencies: vec![("::protobuf::proto",&protobuf::proto::DESCRIPTOR)],
        protobuf_crate: "::protobuf".into(),
    }.generate().expect("generate()");
}
