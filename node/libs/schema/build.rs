//! Generates rust code from the capnp schema files in the `capnp/` directory.
fn main() {
    protobuf::build::Config {
        input_root: "proto".into(),
        proto_root: "zksync/schema".into(),
        dependencies: vec![("::protobuf::proto".into(),&protobuf::proto::DESCRIPTOR)],
        protobuf_crate: "::protobuf".into(),
    }.generate().expect("generate()");
}
