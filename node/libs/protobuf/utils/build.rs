//! Generates rust code from the capnp schema files in the `capnp/` directory.
use std::{env, path::PathBuf};

fn main() -> anyhow::Result<()> {
    // Prepare input and output root dirs.
    let proto_include = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?)
        .canonicalize()?
        .join("proto");
    protobuf_build::compile(&proto_include,"crate::proto",&[])?;
    Ok(())
}
