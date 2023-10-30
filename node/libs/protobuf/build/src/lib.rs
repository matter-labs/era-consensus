//! Generates rust code from the capnp schema files in the `capnp/` directory.
use anyhow::Context as _;
use std::{collections::BTreeMap, env, fs, path::{PathBuf,Path}};
use std::process::Command;

/// Traversed all the files in a directory recursively.
fn traverse_files(path: &Path, f: &mut dyn FnMut(&Path)) -> std::io::Result<()> {
    if !path.is_dir() {
        f(&path);
        return Ok(());
    }
    for entry in fs::read_dir(path)? {
        traverse_files(&entry?.path(), f)?;
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

pub fn compile(proto_include: &Path, module_name: &str, deps: &[(&str,&[u8])]) -> anyhow::Result<()> {
    println!("cargo:rerun-if-changed={}", proto_include.to_str().unwrap());
    let proto_output = PathBuf::from(env::var("OUT_DIR").unwrap())
        .canonicalize()?
        .join("proto");
    let _ = fs::remove_dir_all(&proto_output);
    fs::create_dir_all(&proto_output).unwrap();

    // Find all proto files.
    let mut proto_inputs : Vec<PathBuf> = vec![];
    traverse_files(proto_include, &mut |path| {
        let Some(ext) = path.extension() else { return };
        let Some(ext) = ext.to_str() else { return };
        if ext != "proto" {
            return;
        };
        proto_inputs.push(path.into());
    })?;

    // Compile input files into descriptor.
    let descriptor_path = proto_output.join("descriptor.binpb");
    let mut cmd = Command::new(protoc_bin_vendored::protoc_bin_path().unwrap());
    cmd.arg("-o").arg(&descriptor_path);
    cmd.arg("-I").arg(&proto_include);

    let mut pool = prost_reflect::DescriptorPool::new();
    if deps.len() > 0 {
        let mut deps_list = vec![];
        for (i,(_,d)) in deps.iter().enumerate() {
            pool.decode_file_descriptor_set(*d).unwrap(); // TODO: make it transitive.
            let name = proto_output.join(format!("dep{i}.binpb"));
            fs::write(&name,d)?;
            deps_list.push(name.to_str().unwrap().to_string());
        }
        cmd.arg("--descriptor_set_in").arg(deps_list.join(":"));
    }

    for input in &proto_inputs {
        cmd.arg(&input);
    }

    let out = cmd.output().context("protoc execution failed")?;

    if !out.status.success() {
        anyhow::bail!("protoc_failed:\n{}",String::from_utf8_lossy(&out.stderr));
    }
    
    // Generate protobuf code from schema (with reflection).
    let descriptor = fs::read(&descriptor_path)?;
    pool.decode_file_descriptor_set(&descriptor[..])?;
    let file_descriptor_set_bytes = format!("{module_name}::DESCRIPTOR_POOL");
    let pool_attribute = format!(r#"#[prost_reflect(file_descriptor_set_bytes = "{}")]"#,file_descriptor_set_bytes);
    
    let empty : &[&Path] = &[];
    let mut config = prost_build::Config::new();
    for message in pool.all_messages() {
        let full_name = message.full_name();
        config
            .type_attribute(full_name, "#[derive(::prost_reflect::ReflectMessage)]")
            .type_attribute(full_name, &format!(r#"#[prost_reflect(message_name = "{}")]"#, full_name))
            .type_attribute(full_name, &pool_attribute);
    }
    config.file_descriptor_set_path(&descriptor_path);
    config.skip_protoc_run();
    config.out_dir(&proto_output);
    config.compile_protos(empty,empty).unwrap();

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
    let mut file = deps.iter().map(|d|format!("use {}::*;",d.0)).collect::<Vec<_>>().join("\n");
    file += &m.generate();
    file += &format!("pub const DESCRIPTOR_POOL: &'static [u8] = include_bytes!({descriptor_path:?});");

    let file = syn::parse_str(&file).unwrap();
    fs::write(proto_output.join("mod.rs"), prettyplease::unparse(&file))?;
    Ok(())
}
