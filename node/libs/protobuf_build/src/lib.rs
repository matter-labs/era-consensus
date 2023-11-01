//! Generates rust code from the protobuf
use anyhow::Context as _;
use std::{collections::BTreeMap, fs, path::{PathBuf,Path}};
use std::process::Command;
use prost::Message as _;
use std::collections::HashSet;
use std::sync::Mutex;

pub use prost;
pub use prost_reflect;
pub use once_cell::sync::Lazy;

mod ident;

static POOL : Lazy<Mutex<prost_reflect::DescriptorPool>> = Lazy::new(||Mutex::default());

pub fn global() -> prost_reflect::DescriptorPool { POOL.lock().unwrap().clone() } 

pub struct Descriptor {
    pub module_path: String,
}

impl Descriptor {
    pub fn new(module_path: &str, fds: &impl AsRef<[u8]>) -> Self {
        POOL.lock().unwrap().decode_file_descriptor_set(fds.as_ref()).unwrap();
        Descriptor { module_path: module_path.to_string() }
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
    modules: BTreeMap<String, Module>,
    /// Code of the module.
    code: String,
}

impl Module {
    /// Inserts a nested generated protobuf module.
    /// `name` is a sequence of module names.
    fn insert<'a>(&mut self, mut name: impl Iterator<Item=&'a str>, code: &str) {
        match name.next() {
            None => self.code += code,
            Some(module) => self.modules.entry(module.to_string()).or_default().insert(name,code),
        }
    }

    /// Generates rust code of the module.
    fn generate(&self) -> String {
        let mut entries = vec![self.code.clone()];
        entries.extend(
            self.modules.iter().map(|(name, m)| format!("pub mod {name} {{ {} }}\n", m.generate())),
        );
        entries.join("")
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


pub struct Config {
    pub input_root: PathBuf,
    pub dependencies: Vec<&'static Lazy<Descriptor>>,
    
    pub output_mod_path: PathBuf,
    pub output_descriptor_path: PathBuf,

    pub protobuf_crate: String,
}

impl Config {
    fn import(&self, elem: &str) -> String {
        format!("{}::build::{}",self.protobuf_crate,elem)
    }

    fn reflect_impl(&self, m: prost_reflect::MessageDescriptor) -> String {
        let proto_name = m.full_name();
        let parts : Vec<&str> = proto_name.split(".").collect();
        let mut rust_name : Vec<_> = parts[0..parts.len()-1].iter().map(|p|ident::to_snake(*p)).collect();
        rust_name.push(ident::to_upper_camel(parts[parts.len()-1]));
        let rust_name = rust_name.join("::");
        let rust_reflect = self.import("prost_reflect");
        let rust_global = self.import("global");
        format!("impl {rust_reflect}::ReflectMessage for {rust_name} {{ \
            fn descriptor(&self) -> {rust_reflect}::MessageDescriptor {{ \
                &*DESCRIPTOR; \
                {rust_global}() \
                    .get_message_by_name(\"{proto_name}\") \
                    .expect(\"descriptor for message type {proto_name} not found\") \
            }}\
        }}")
    }


    pub fn generate(&self) -> anyhow::Result<()> { 
        println!("cargo:rerun-if-changed={:?}", self.input_root);
        
        // Find all proto files.
        let mut inputs : Vec<PathBuf> = vec![];
        traverse_files(&self.input_root, &mut |path| {
            let Some(ext) = path.extension() else { return };
            let Some(ext) = ext.to_str() else { return };
            if ext != "proto" {
                return;
            };
            inputs.push(path.into());
        })?;

        // Compile input files into descriptor.
        let descriptor_path = &self.output_descriptor_path; //.canonicalize().context("output_descriptor_path.canonicalize()")?;
        fs::create_dir_all(&descriptor_path.parent().unwrap()).unwrap();
        let mut cmd = Command::new(protoc_bin_vendored::protoc_bin_path().unwrap());
        cmd.arg("-o").arg(&descriptor_path);
        cmd.arg("-I").arg(&self.input_root.canonicalize()?);

        /*if deps.len() > 0 {
            let mut deps_list = vec![];
            for (i,(_,d)) in deps.iter().enumerate() {
                let name = proto_output.join(format!("dep{i}.binpb"));
                fs::write(&name,d)?;
                deps_list.push(name.to_str().unwrap().to_string());
            }
            cmd.arg("--descriptor_set_in").arg(deps_list.join(":"));
        }*/

        let deps : Vec<_> = self.dependencies.iter().map(|d|&***d).collect();
        let deps_path = "/tmp/deps.binpb";
        fs::write(deps_path, global().encode_to_vec())?;
        cmd.arg("--descriptor_set_in").arg(deps_path);

        for input in &inputs { cmd.arg(&input); }

        let out = cmd.output().context("protoc execution failed")?;

        if !out.status.success() {
            anyhow::bail!("protoc_failed:\n{}",String::from_utf8_lossy(&out.stderr));
        }
        
        // Generate protobuf code from schema.
        let mut config = prost_build::Config::new();
        config.prost_path(self.import("prost"));
        config.file_descriptor_set_path(&descriptor_path);
        config.skip_protoc_run();
        
        // Check that messages are compatible with `proto_fmt::canonical`.
        let descriptor = fs::read(&descriptor_path)?;
        let descriptor = prost_types::FileDescriptorSet::decode(&descriptor[..]).unwrap();
        let new_messages = get_messages(descriptor.clone(),global());
        
        let mut check_state = CanonicalCheckState::default();
        for m in &new_messages {
            check_state.check_message(m,false)?;
        }

        let modules : Vec<_> = descriptor.file.iter().map(|d|(prost_build::Module::from_protobuf_package_name(d.package()),d.clone())).collect();
        let mut m = Module::default();
        for (name,code) in config.generate(modules).unwrap() {
            m.insert(name.parts(),&code);
        }
        let mut file = deps.iter().map(|d|format!("use {}::*;",d.module_path)).collect::<Vec<_>>().join("\n");
        file += &m.generate(); 
        for m in &new_messages {
            file += &self.reflect_impl(m.clone());
        }

        let rec = deps.iter().map(|d|format!("&*{}::DESCRIPTOR;",d.module_path)).collect::<Vec<_>>().join(" ");
        let rust_lazy = self.import("Lazy");
        let rust_descriptor = self.import("Descriptor");
        file += &format!("\
            pub static DESCRIPTOR : {rust_lazy}<{rust_descriptor}> = {rust_lazy}::new(|| {{\
                {rec}
                {rust_descriptor}::new(module_path!(), &include_bytes!({descriptor_path:?}))\
            }});\
        ");

        let file = syn::parse_str(&file).unwrap();
        fs::write(&self.output_mod_path, prettyplease::unparse(&file))?;
        Ok(())
    }
}
