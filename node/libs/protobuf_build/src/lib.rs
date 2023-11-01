//! Generates rust code from the protobuf
use anyhow::Context as _;
use std::{collections::BTreeMap, fs, path::{PathBuf,Path}};
use std::process::Command;
use prost::Message as _;
use std::collections::HashSet;
use std::sync::Mutex;

pub use prost;
pub use prost_reflect;
pub use once_cell::sync::{Lazy};

mod ident;

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

fn get_messages(fds: &prost_types::FileDescriptorSet, pool: &prost_reflect::DescriptorPool) -> Vec<prost_reflect::MessageDescriptor> {
    let mut res = vec![];
    for f in &fds.file {
        get_messages_from_file(&mut res,pool.get_file_by_name(f.name()).unwrap());
    }
    res
}

pub struct Descriptor {
    pub rust_module : String,
    pub proto_package : String,
    pub descriptor_proto : prost_types::FileDescriptorSet,
    pub dependencies : Vec<&'static Descriptor>,
}

impl Descriptor {
    pub fn new(rust_module: &str, proto_package: &str, dependencies: Vec<&'static Descriptor>, descriptor_bytes: &impl AsRef<[u8]>) -> Self {
        Descriptor {
            rust_module: rust_module.to_string(),
            proto_package: proto_package.to_string(),
            dependencies,
            descriptor_proto: prost_types::FileDescriptorSet::decode(descriptor_bytes.as_ref()).unwrap(),
        }
    }

    pub fn load(&self, pool: &mut prost_reflect::DescriptorPool) -> anyhow::Result<()> {
        if self.descriptor_proto.file.iter().all(|f| pool.get_file_by_name(f.name()).is_some()) {
            return Ok(());
        }
        for d in &self.dependencies {
            d.load(pool)?;
        }
        pool.add_file_descriptor_set(self.descriptor_proto.clone())?;
        Ok(())
    }

    pub fn load_global(&self) -> prost_reflect::DescriptorPool {
        static POOL : Lazy<Mutex<prost_reflect::DescriptorPool>> = Lazy::new(||Mutex::default());
        let pool = &mut POOL.lock().unwrap();
        self.load(pool).unwrap();
        pool.clone()
    }
}

pub struct Config {
    pub proto_path: PathBuf,
    pub proto_package: String,
    pub dependencies: Vec<&'static Descriptor>,
    
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
        let rust_lazy = self.import("Lazy");
        format!("impl {rust_reflect}::ReflectMessage for {rust_name} {{\
            fn descriptor(&self) -> {rust_reflect}::MessageDescriptor {{\
                static INIT : {rust_lazy}<{rust_reflect}::MessageDescriptor> = {rust_lazy}::new(|| {{\
                    DESCRIPTOR.load_global().get_message_by_name(\"{proto_name}\").unwrap()\
                }});\
                INIT.clone()\
            }}\
        }}")
    }

    pub fn generate(&self) -> anyhow::Result<()> { 
        println!("cargo:rerun-if-changed={:?}", self.proto_path);
        
        // Find all proto files.
        let mut inputs : Vec<PathBuf> = vec![];
        traverse_files(&self.proto_path, &mut |path| {
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
        cmd.arg("-I").arg(&self.proto_path.canonicalize()?);

        let mut pool = prost_reflect::DescriptorPool::new();
        for d in &self.dependencies { d.load(&mut pool).unwrap(); }
        let pool_path = "/tmp/pool.binpb";
        fs::write(pool_path, pool.encode_to_vec())?;
        cmd.arg("--descriptor_set_in").arg(pool_path);
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
        for d in &self.dependencies {
            for f in &d.descriptor_proto.file {
                config.extern_path(format!(".{}",f.package()), format!("{}::{}",&d.rust_module,f.package().split(".").map(ident::to_snake).collect::<Vec<_>>().join("::")));
            }
        }

        // Check that messages are compatible with `proto_fmt::canonical`.
        let descriptor = fs::read(&descriptor_path)?;
        let descriptor = prost_types::FileDescriptorSet::decode(&descriptor[..]).unwrap();
        for f in &descriptor.file {
            if !f.package().starts_with(&self.proto_package) {
                anyhow::bail!("{:?} ({:?}) does not belong to package {:?}",f.package(),f.name(),self.proto_package);
            }
        }
        pool.add_file_descriptor_set(descriptor.clone()).unwrap();
        let new_messages = get_messages(&descriptor,&pool);
        
        let mut check_state = CanonicalCheckState::default();
        for m in &new_messages {
            check_state.check_message(m,false)?;
        }

        let modules : Vec<_> = descriptor.file.iter().map(|d|(prost_build::Module::from_protobuf_package_name(d.package()),d.clone())).collect();
        let mut m = Module::default();
        for (name,code) in config.generate(modules).unwrap() {
            m.insert(name.parts(),&code);
        }
        let mut file = m.generate(); 
        for m in &new_messages {
            file += &self.reflect_impl(m.clone());
        }

        let rust_deps = self.dependencies.iter().map(|d|format!("&::{}::DESCRIPTOR",d.rust_module)).collect::<Vec<_>>().join(",");
        let rust_lazy = self.import("Lazy");
        let rust_descriptor = self.import("Descriptor");
        file += &format!("\
            pub static DESCRIPTOR : {rust_lazy}<{rust_descriptor}> = {rust_lazy}::new(|| {{\
                {rust_descriptor}::new(module_path!(), {:?}, vec![{rust_deps}], &include_bytes!({descriptor_path:?}))\
            }});\
        ",self.proto_package);

        let file = syn::parse_str(&file).unwrap();
        fs::write(&self.output_mod_path, prettyplease::unparse(&file))?;
        Ok(())
    }
}
