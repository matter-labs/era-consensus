//! Generates rust code from the capnp schema files in the `capnp/` directory.
use anyhow::Context as _;
use std::{collections::BTreeMap, env, fs, path::PathBuf};

/// Traversed all the files in a directory recursively.
fn traverse_files(path: PathBuf, f: &mut dyn FnMut(PathBuf)) -> std::io::Result<()> {
    if !path.is_dir() {
        f(path);
        return Ok(());
    }
    for entry in fs::read_dir(path)? {
        traverse_files(entry?.path(), f)?;
    }
    Ok(())
}

/// A rust module representation.
/// It is used to collect the generated protobuf code.
#[derive(Default)]
struct Module {
    /// Nested modules which transitively contain the generated code.
    nested: BTreeMap<String, Module>,
    /// Nested modules directly contains the generated code.
    include: BTreeMap<String, PathBuf>,
}

impl Module {
    /// Inserts a nested generated protobuf module.
    /// `name` is a sequence of module names.
    fn insert(&mut self, name: &[String], file: PathBuf) {
        println!(" -- {name:?}");
        match name.len() {
            0 => panic!("empty module path"),
            1 => assert!(
                self.include.insert(name[0].clone(), file).is_none(),
                "duplicate module"
            ),
            _ => self
                .nested
                .entry(name[0].clone())
                .or_default()
                .insert(&name[1..], file),
        }
    }

    /// Generates rust code of the module.
    fn generate(&self) -> String {
        let mut entries = vec![];
        entries.extend(
            self.nested
                .iter()
                .map(|(name, m)| format!("pub mod {name} {{ {} }}", m.generate())),
        );
        entries.extend(
            self.include
                .iter()
                .map(|(name, path)| format!("pub mod {name} {{ include!({path:?}); }}",)),
        );
        entries.join("\n")
    }
}

/// Checks if field `f` supports canonical encoding.
fn check_canonical_field(f: &prost_reflect::FieldDescriptor) -> anyhow::Result<()> {
    if f.is_map() {
        anyhow::bail!("maps unsupported");
    }
    if !f.is_list() && !f.supports_presence() {
        anyhow::bail!("non-repeated, non-oneof fields have to be marked as optional");
    }
    Ok(())
}

/// Checks if messages of type `m` support canonical encoding.
fn check_canonical_message(m: &prost_reflect::MessageDescriptor) -> anyhow::Result<()> {
    for m in m.child_messages() {
        check_canonical_message(&m).with_context(|| m.name().to_string())?;
    }
    for f in m.fields() {
        check_canonical_field(&f).with_context(|| f.name().to_string())?;
    }
    Ok(())
}

/// Checks if message types in file `f` support canonical encoding.
fn check_canonical_file(f: &prost_reflect::FileDescriptor) -> anyhow::Result<()> {
    if f.syntax() != prost_reflect::Syntax::Proto3 {
        anyhow::bail!("only proto3 syntax is supported");
    }
    for m in f.messages() {
        check_canonical_message(&m).with_context(|| m.name().to_string())?;
    }
    Ok(())
}

/// Checks if message types in descriptor pool `d` support canonical encoding.
fn check_canonical_pool(d: &prost_reflect::DescriptorPool) -> anyhow::Result<()> {
    for f in d.files() {
        check_canonical_file(&f).with_context(|| f.name().to_string())?;
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    // Prepare input and output root dirs.
    let proto_include = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?)
        .canonicalize()?
        .join("proto");
    let proto_output = PathBuf::from(env::var("OUT_DIR").unwrap())
        .canonicalize()?
        .join("proto");
    println!("cargo:rerun-if-changed={}", proto_include.to_str().unwrap());
    let _ = fs::remove_dir_all(&proto_output);
    fs::create_dir_all(&proto_output).unwrap();

    // Find all proto files.
    let mut proto_inputs = vec![];
    traverse_files(proto_include.clone(), &mut |path| {
        let Some(ext) = path.extension() else { return };
        let Some(ext) = ext.to_str() else { return };
        if ext != "proto" {
            return;
        };
        proto_inputs.push(path);
    })?;

    // Generate protobuf code from schema (with reflection).
    env::set_var("PROTOC", protoc_bin_vendored::protoc_bin_path().unwrap());
    let mut config = prost_build::Config::new();
    let descriptor_path = proto_output.join("descriptor.bin");
    config.out_dir(&proto_output);
    prost_reflect_build::Builder::new()
        .file_descriptor_set_path(&descriptor_path)
        .descriptor_pool("crate::proto::DESCRIPTOR_POOL")
        .compile_protos_with_config(config, &proto_inputs, &[&proto_include])
        .unwrap();
    let descriptor = fs::read(descriptor_path)?;
    let pool = prost_reflect::DescriptorPool::decode(descriptor.as_ref()).unwrap();

    // Check that messages are compatible with `proto_fmt::canonical`.
    check_canonical_pool(&pool)?;

    // Generate mod file collecting all proto-generated code.
    let mut m = Module::default();
    for entry in fs::read_dir(&proto_output).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().into_string().unwrap();
        let Some(name) = name.strip_suffix(".rs") else {
            continue;
        };
        let name: Vec<_> = name.split('.').map(String::from).collect();
        println!("name = {name:?}");
        m.insert(&name, entry.path());
    }
    let file = m.generate();
    let file = syn::parse_str(&file).unwrap();
    fs::write(proto_output.join("mod.rs"), prettyplease::unparse(&file))?;
    Ok(())
}
