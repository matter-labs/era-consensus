//! Generates rust code from the protobuf
//!
//! cargo build --all-targets
//! perl -ne 'print "$1\n" if /PROTOBUF_DESCRIPTOR="(.*)"/' `find target/debug/build/*/output -type f` | xargs cat > /tmp/sum.binpb
//! buf breaking /tmp/sum.binpb --against /tmp/sum.binpb
use anyhow::Context as _;
use std::{collections::BTreeMap, fs, path::{PathBuf,Path}};
use prost::Message as _;
use std::collections::HashSet;
use std::sync::Mutex;

pub use prost;
pub use prost_reflect;
pub use once_cell::sync::{Lazy};

mod ident;

/// Traversed all the files in a directory recursively.
fn traverse_files(path: &Path, f: &mut impl FnMut(&Path) -> anyhow::Result<()>) -> anyhow::Result<()> {
    if !path.is_dir() {
        f(path).with_context(||path.to_str().unwrap().to_string())?;
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
    pub proto_package: ProtoPackage,
    pub descriptor_proto: prost_types::FileDescriptorSet,
    pub dependencies: Vec<&'static Descriptor>,
}

impl Descriptor {
    pub fn new(proto_package: ProtoPackage, dependencies: Vec<&'static Descriptor>, descriptor_bytes: &impl AsRef<[u8]>) -> Self {
        Descriptor {
            proto_package,
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

#[derive(Clone,PartialEq,Eq)]
pub struct RustName(Vec<String>);

impl RustName {
    fn add(mut self, suffix: impl Into<Self>) -> Self {
        self.0.extend(suffix.into().0.into_iter());
        self
    }

    fn to_string(&self) -> String { self.0.join("::") }
}

impl From<&str> for RustName {
    fn from(s:&str) -> Self { Self(s.split("::").map(String::from).collect()) }
}

#[derive(Clone,PartialEq,Eq)]
pub struct ProtoPackage(Vec<String>);

impl ProtoPackage {
    fn starts_with(&self, prefix: &Self) -> bool {
        let n = prefix.0.len();
        self.0.len() >= n && self.0[0..n] == prefix.0
    }

    fn strip_prefix(&self, prefix: &Self) -> anyhow::Result<Self> {
        if !self.starts_with(prefix) {
            anyhow::bail!("{} is not a prefix of {}",prefix.to_string(),self.to_string());
        }
        Ok(Self(self.0[prefix.0.len()..].iter().cloned().collect()))
    }

    fn to_rust_module(&self) -> RustName {
        RustName(self.0.iter().map(|s|ident::to_snake(s)).collect())
    }

    fn to_rust_type(&self) -> RustName {
        let n = self.0.len();
        let mut res = self.to_rust_module();
        res.0[n-1] = ident::to_upper_camel(&self.0[n-1]);
        res
    }

    fn to_string(&self) -> String {
        self.0.join(".")
    }
}

impl From<&str> for ProtoPackage {
    fn from(s:&str) -> Self {
        Self(s.split(".").map(String::from).collect())
    }
}

pub struct Config {
    pub input_path: PathBuf,
    pub proto_path: PathBuf,
    pub proto_package: ProtoPackage,
    pub dependencies: Vec<(&'static str, &'static Descriptor)>,
    
    pub output_mod_path: PathBuf,
    pub output_descriptor_path: PathBuf,

    pub protobuf_crate: RustName,
}

impl Config {
    fn this_crate(&self) -> RustName {
        self.protobuf_crate.clone().add("build")
    }

    fn reflect_impl(&self, m: prost_reflect::MessageDescriptor) -> String {
        let proto_name = m.full_name();
        let rust_name = ProtoPackage::from(proto_name).strip_prefix(&self.proto_package).unwrap().to_rust_type().to_string();
        let this = self.this_crate().to_string();
        format!("impl {this}::prost_reflect::ReflectMessage for {rust_name} {{\
            fn descriptor(&self) -> {this}::prost_reflect::MessageDescriptor {{\
                static INIT : {this}::Lazy<{this}::prost_reflect::MessageDescriptor> = {this}::Lazy::new(|| {{\
                    DESCRIPTOR.load_global().get_message_by_name({proto_name:?}).unwrap()\
                }});\
                INIT.clone()\
            }}\
        }}")
    }

    pub fn generate(&self) -> anyhow::Result<()> { 
        println!("cargo:rerun-if-changed={:?}", self.input_path);
        
        // Find all proto files.
        let mut pool = prost_reflect::DescriptorPool::new();
        for d in &self.dependencies { d.1.load(&mut pool).unwrap(); }
        let mut x = prost_types::FileDescriptorSet::default();
        x.file = pool.file_descriptor_protos().cloned().collect();
        let input_path = self.input_path.canonicalize()?;
        let mut new_paths = vec![];
        traverse_files(&input_path, &mut |path| {
            let Some(ext) = path.extension() else { return Ok(()) };
            let Some(ext) = ext.to_str() else { return Ok(()) };
            if ext != "proto" { return Ok(()) };
            let rel_path = self.proto_path.join(path.strip_prefix(&input_path).unwrap());
            x.file.push(protox_parse::parse(
                rel_path.to_str().unwrap(),
                &fs::read_to_string(path).context("fs::read()")?,
            ).context("protox_parse::parse()")?);
            new_paths.push(rel_path.clone());
            Ok(())
        })?;
        let mut compiler = protox::Compiler::with_file_resolver(
            protox::file::DescriptorSetFileResolver::new(x)
        );
        // TODO: nice compilation errors.
        compiler.open_files(new_paths).unwrap();
        let descriptor = compiler.file_descriptor_set();
        pool.add_file_descriptor_set(descriptor.clone()).unwrap();
        fs::create_dir_all(&self.output_descriptor_path.parent().unwrap()).unwrap();
        fs::write(&self.output_descriptor_path, &descriptor.encode_to_vec())?;
        
        // Generate protobuf code from schema.
        let mut config = prost_build::Config::new();
        config.prost_path(self.this_crate().add("prost").to_string());
        config.skip_protoc_run();
        for d in &self.dependencies {
            for f in &d.1.descriptor_proto.file {
                let proto_rel = ProtoPackage::from(f.package()).strip_prefix(&d.1.proto_package).unwrap();
                let rust_abs = RustName::from(d.0).add(proto_rel.to_rust_module());
                config.extern_path(format!(".{}",f.package()), rust_abs.to_string());
            }
        }

        // Check that messages are compatible with `proto_fmt::canonical`.
        for f in &descriptor.file {
            if !ProtoPackage::from(f.package()).starts_with(&self.proto_package) {
                anyhow::bail!("{:?} ({:?}) does not belong to package {:?}",f.package(),f.name(),self.proto_package.to_string());
            }
        } 
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
        let mut m = &m;
        for part in &self.proto_package.0 { m = &m.modules[part]; }
        let mut file = m.generate(); 
        for m in &new_messages {
            file += &self.reflect_impl(m.clone());
        }

        let rust_deps = self.dependencies.iter().map(|d|format!("&{}::DESCRIPTOR",d.0.to_string())).collect::<Vec<_>>().join(",");
        let this = self.this_crate().to_string();
        file += &format!("\
            pub static DESCRIPTOR : {this}::Lazy<{this}::Descriptor> = {this}::Lazy::new(|| {{\
                {this}::Descriptor::new({:?}.into(), vec![{rust_deps}], &include_bytes!({:?}))\
            }});\
        ",self.proto_package.to_string(),self.output_descriptor_path);

        let file = syn::parse_str(&file).unwrap();
        fs::write(&self.output_mod_path, prettyplease::unparse(&file))?;
        println!("PROTOBUF_DESCRIPTOR={:?}",self.output_descriptor_path);
        Ok(())
    }
}
