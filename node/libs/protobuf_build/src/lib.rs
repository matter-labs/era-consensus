//! Generates rust code from the protobuf
use anyhow::Context as _;
use std::{collections::BTreeMap, env, fs, path::{PathBuf,Path}};
use std::process::Command;
use prost::Message as _;
use std::collections::HashSet;
pub use once_cell::sync::Lazy;
use std::sync::Mutex;

static POOL : Lazy<Mutex<prost_reflect::DescriptorPool>> = Lazy::new(||Mutex::default());

pub fn global(_:()) -> prost_reflect::DescriptorPool { POOL.lock().unwrap().clone() } 

pub type LazyDescriptor = Lazy<Descriptor>;

pub struct Descriptor {
    pub module_path: String,
}

impl Descriptor {
    pub fn new(module_path: String, fds: &impl AsRef<[u8]>) -> Self {
        POOL.lock().unwrap().decode_file_descriptor_set(fds.as_ref()).unwrap();
        Descriptor { module_path }
    }

    pub fn load(&self) {}
}

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
                .map(|(name, m)| format!("pub mod {name} {{ use super::{{protobuf,DESCRIPTOR}}; {} }}", m.generate())),
        );
        entries.extend(
            self.include
                .iter()
                .map(|(name, path)| format!("pub mod {name} {{ use super::{{protobuf,DESCRIPTOR}}; include!({path:?}); }}",)),
        );
        entries.join("\n")
    }
}

#[derive(Default)]
struct CanonicalCheckState(HashSet<String>);

impl CanonicalCheckState {
    /// Checks if messages of type `m` support canonical encoding.
    fn check_message(&mut self, m: &prost_reflect::MessageDescriptor, check_nested: bool) -> anyhow::Result<()> { 
        if self.0.contains(m.full_name()) {
            return Ok(());
        }
        self.0.insert(m.full_name().to_string());
        for f in m.fields() {
            self.check_field(&f).with_context(|| f.name().to_string())?;
        }
        if check_nested {
            for m in m.child_messages() {
                self.check_message(&m,check_nested).with_context(||m.name().to_string())?;
            }
        }
        Ok(())
    }

    /// Checks if field `f` supports canonical encoding.
    fn check_field(&mut self, f: &prost_reflect::FieldDescriptor) -> anyhow::Result<()> {
        if f.is_map() {
            anyhow::bail!("maps unsupported");
        }
        if !f.is_list() && !f.supports_presence() {
            anyhow::bail!("non-repeated, non-oneof fields have to be marked as optional");
        }
        if let prost_reflect::Kind::Message(msg) = f.kind() {
            self.check_message(&msg,false).with_context(||msg.name().to_string())?;
        }
        Ok(())
    }

    /*/// Checks if message types in file `f` support canonical encoding.
    fn check_file(&mut self, f: &prost_reflect::FileDescriptor) -> anyhow::Result<()> {
        if f.syntax() != prost_reflect::Syntax::Proto3 {
            anyhow::bail!("only proto3 syntax is supported");
        }
        for m in f.messages() {
            self.check_message(&m,true).with_context(|| m.name().to_string())?;
        }
        Ok(())
    }*/
}

fn get_messages_from_message(out: &mut Vec<prost_reflect::MessageDescriptor>, m: prost_reflect::MessageDescriptor) {
    for m in m.child_messages() {
        get_messages_from_message(out,m);
    }
    out.push(m);
}

fn get_messages_from_file(out: &mut Vec<prost_reflect::MessageDescriptor>, f: prost_reflect::FileDescriptor) {
    for m in f.messages() {
        get_messages_from_message(out,m);
    }
}

fn get_messages(fds: prost_types::FileDescriptorSet, mut pool: prost_reflect::DescriptorPool) -> Vec<prost_reflect::MessageDescriptor> {
    let mut res = vec![];
    pool.add_file_descriptor_set(fds.clone()).unwrap();
    for f in fds.file {
        get_messages_from_file(&mut res,pool.get_file_by_name(f.name.as_ref().unwrap()).unwrap());
    }
    res
}

/*
fn reflect_impl_msg(m: prost_reflect::MessageDescriptor) -> String {
    let full_name = m.full_name(); 
    let mut res = format!("impl prost_reflect::ReflectMessage for {full_name} {{ \
        fn descriptor(&self) -> ::prost_reflect::MessageDescriptor {{ \
            &*DESCRIPTOR; \
            protobuf::build::global() \
                .get_message_by_name(\"{full_name}\") \
                .expect(\"descriptor for message type {full_name} not found\") \
        }}\
    }}");
    for m in m.child_messages() {
        res += &reflect_impl_msg(m);
    }
    res
}

fn reflect_impl_file(f: prost_reflect::FileDescriptor) -> String {
    f.messages().map(reflect_impl_msg).collect::<Vec<_>>().join("")
}*/

pub fn compile(proto_include: &Path, deps: &[&Lazy<Descriptor>]) -> anyhow::Result<()> {
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

    /*if deps.len() > 0 {
        let mut deps_list = vec![];
        for (i,(_,d)) in deps.iter().enumerate() {
            let name = proto_output.join(format!("dep{i}.binpb"));
            fs::write(&name,d)?;
            deps_list.push(name.to_str().unwrap().to_string());
        }
        cmd.arg("--descriptor_set_in").arg(deps_list.join(":"));
    }*/

    let deps : Vec<_> = deps.iter().map(|d|&***d).collect();
    let deps_path = proto_output.join("deps.binpb");
    fs::write(&deps_path,prost_reflect::DescriptorPool::global().encode_to_vec())?;
    cmd.arg("--descriptor_set_in").arg(&deps_path);

    for input in &proto_inputs {
        cmd.arg(&input);
    }

    let out = cmd.output().context("protoc execution failed")?;

    if !out.status.success() {
        anyhow::bail!("protoc_failed:\n{}",String::from_utf8_lossy(&out.stderr));
    }
    
    // Generate protobuf code from schema.
    let empty : &[&Path] = &[];
    let mut config = prost_build::Config::new();
    config.file_descriptor_set_path(&descriptor_path);
    config.skip_protoc_run();
    config.out_dir(&proto_output);
    
    // Check that messages are compatible with `proto_fmt::canonical`.
    let descriptor = fs::read(&descriptor_path)?;
    let descriptor = prost_types::FileDescriptorSet::decode(&descriptor[..]).unwrap();
    let new_messages = get_messages(descriptor,global(()));
    
    let mut check_state = CanonicalCheckState::default();
    for m in &new_messages {
        check_state.check_message(m,false)?;
    }

    let pool_attribute = format!("#[prost_reflect(descriptor_pool = \"protobuf::build::global(DESCRIPTOR.load())\")]");
    for m in &new_messages {
        let full_name = m.full_name();
        config.type_attribute(full_name, "#[derive(::prost_reflect::ReflectMessage)]");
        config.type_attribute(full_name, &format!("#[prost_reflect(message_name = {full_name:?})]"));
        config.type_attribute(full_name, &pool_attribute);
    }

    config.compile_protos(empty,empty).unwrap();

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
    let mut file = deps.iter().map(|d|format!("use {}::*;",d.module_path)).collect::<Vec<_>>().join("\n");
    file += &m.generate();  
    
    let rec = deps.iter().map(|d|format!("&*{}::DESCRIPTOR;",d.module_path)).collect::<Vec<_>>().join(" ");
    file += &format!("\
        static DESCRIPTOR : protobuf::build::LazyDescriptor = protobuf::build::Lazy::new(|| {{\
            {rec}
            protobuf::build::Descriptor::new(module_path!(), &include_bytes!({descriptor_path:?}))\
        }});\
    ");

    //fs::write(proto_output.join("mod.rs"), file)?;
    let file = syn::parse_str(&file).unwrap();
    fs::write(proto_output.join("mod.rs"), prettyplease::unparse(&file))?;
    Ok(())
}
