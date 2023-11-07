//! Generates rust code from the proto files.
//!
//! Protobuf files are collected recursively from $CARGO_MANIFEST_DIR/<input_root>/ directory.
//! Corresponding "cargo:rerun-if-changed=..." line is printed to stdout, so that
//! the build script running this function is rerun whenever proto files change.
//! A single rust file is generated and stored at $OUT_DIR/<input_root>/gen.rs file.
//!
//! Protobuf files are compiled to a protobuf descriptor stored at
//! $OUT_DIR/<input_root>/gen.binpb.
//! Additionally a "PROTOBUF_DESCRIPTOR=<absolute path to that descriptor>" line is printed to
//! stdout. This can be used to collect all the descriptors across the build as follows:
//! 1. Checkout the repo to a fresh directory and then run "cargo build --all-targets"
//!    We need it fresh so that every crate containing protobufs has only one build in the
//!    cargo cache.
//! 2. grep through all target/debug/build/*/output files to find all "PROTOBUF_DESCRIPTOR=..."
//!    lines and merge the descriptor files by simply concatenating them.
//! Note that you can run this procedure for 2 revisions of the repo and look for breaking
//! changes by running "buf breaking <after.binpb> --against <before.binpb>" where before.binpb
//! and after.binpb are the concatenated descriptors from those 2 revisions.
//!
//! The proto files are not expected to be self-contained - to import proto files from
//! different crates you need to specify them as dependencies in the Config.dependencies.
//! It is not possible to depend on a different proto bundle within the same crate (because
//! these are being built simultaneously from the same build script).
#![allow(clippy::print_stdout)]
use anyhow::Context as _;
use prost::Message as _;
use std::{fs, path::Path, sync::Mutex};

// Imports accessed from the generated code.
pub use once_cell::sync::Lazy;
pub use prost;
pub use prost_reflect;
pub use self::syntax::*;

mod canonical;
mod ident;
mod syntax;

/// Traverses all the files in a directory recursively.
fn traverse_files(
    path: &Path,
    f: &mut impl FnMut(&Path) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    if !path.is_dir() {
        f(path).with_context(|| path.display().to_string())?;
        return Ok(());
    }
    for entry in fs::read_dir(path)? {
        traverse_files(&entry?.path(), f)?;
    }
    Ok(())
}

/// Protobuf descriptor + info about the mapping to rust code.
pub struct Descriptor {
    /// Root proto package that all proto files in this descriptor belong to.
    /// Rust types have been generated relative to this root.
    proto_root: ProtoName,
    /// Raw descriptor proto.
    descriptor_proto: prost_types::FileDescriptorSet,
    /// Direct dependencies of this descriptor.
    dependencies: Vec<&'static Descriptor>,
}

impl Descriptor {
    /// Constructs a Descriptor.
    pub fn new(
        proto_root: ProtoName,
        dependencies: Vec<&'static Descriptor>,
        descriptor_bytes: &[u8],
    ) -> Self {
        Descriptor {
            proto_root,
            dependencies,
            descriptor_proto: prost_types::FileDescriptorSet::decode(descriptor_bytes.as_ref())
                .unwrap(),
        }
    }

    /// Loads the descriptor to the pool, if not already loaded.
    pub fn load(&self, pool: &mut prost_reflect::DescriptorPool) -> anyhow::Result<()> {
        if self
            .descriptor_proto
            .file
            .iter()
            .all(|f| pool.get_file_by_name(f.name()).is_some())
        {
            return Ok(());
        }
        for d in &self.dependencies {
            d.load(pool)?;
        }
        pool.add_file_descriptor_set(self.descriptor_proto.clone())?;
        Ok(())
    }

    /// Loads the descriptor to the global pool and returns a copy of the global pool.
    pub fn load_global(&self) -> prost_reflect::DescriptorPool {
        /// Global descriptor pool.
        static POOL: Lazy<Mutex<prost_reflect::DescriptorPool>> = Lazy::new(Mutex::default);
        let pool = &mut POOL.lock().unwrap();
        self.load(pool).unwrap();
        pool.clone()
    }
}

/// Code generation config. Use it in build scripts.
pub struct Config {
    /// Input directory relative to $CARGO_MANIFEST_DIR with the proto files to be compiled.
    pub input_root: InputPath,
    /// Implicit prefix that should be prepended to proto paths of the proto files in the input directory.
    pub proto_root: ProtoPath,
    /// Descriptors of the dependencies and the rust absolute paths under which they will be available from the generated code.
    pub dependencies: Vec<(RustName, &'static Descriptor)>,
    /// Rust absolute path under which the protobuf crate will be available from the generated
    /// code.
    pub protobuf_crate: RustName,
}

impl Config {
    /// Location of the protobuf_build crate, visible from the generated code.
    fn this_crate(&self) -> RustName {
        self.protobuf_crate.clone().join("build")
    }

    /// Generates implementation of `prost_reflect::ReflectMessage` for a rust type generated
    /// from a message of the given `proto_name`.
    fn reflect_impl(&self, proto_name: &ProtoName) -> anyhow::Result<String> {
        let rust_name = proto_name
            .relative_to(&self.proto_root.to_name().context("invalid proto_root")?)
            .unwrap()
            .to_rust_type()
            .to_string();
        let proto_name = proto_name.to_string();
        let this = self.this_crate().to_string();
        Ok(format!("impl {this}::prost_reflect::ReflectMessage for {rust_name} {{\
            fn descriptor(&self) -> {this}::prost_reflect::MessageDescriptor {{\
                static INIT : {this}::Lazy<{this}::prost_reflect::MessageDescriptor> = {this}::Lazy::new(|| {{\
                    DESCRIPTOR.load_global().get_message_by_name({proto_name:?}).unwrap()\
                }});\
                INIT.clone()\
            }}\
        }}"))
    }

    /// Generates rust code from the proto files according to the config.
    pub fn generate(&self) -> anyhow::Result<()> {
        if !self.input_root.abs()?.is_dir() {
            anyhow::bail!("input_root should be a directory");
        }
        println!("cargo:rerun-if-changed={}", self.input_root.to_str());

        // Load dependencies.
        let mut pool = prost_reflect::DescriptorPool::new();
        for d in &self.dependencies {
            d.1.load(&mut pool)
                .with_context(|| format!("failed to load dependency {}", d.0))?;
        }
        let mut pool_raw = prost_types::FileDescriptorSet::default();
        pool_raw.file.extend(pool.file_descriptor_protos().cloned());

        // Load proto files.
        let mut proto_paths = vec![];
        traverse_files(&self.input_root.abs()?, &mut |path| {
            let Some(ext) = path.extension() else {
                return Ok(());
            };
            let Some(ext) = ext.to_str() else {
                return Ok(());
            };
            if ext != "proto" {
                return Ok(());
            };

            let file_raw = fs::read_to_string(path).context("fs::read()")?;
            let path = ProtoPath::from_input_path(path, &self.input_root, &self.proto_root).context("ProtoPath::from_input_path()")?;
            pool_raw
                .file
                .push(protox_parse::parse(&path.to_string(), &file_raw).map_err(
                    // rewrapping the error, so that source location is included in the error message.
                    |err| anyhow::anyhow!("{err:?}"),
                )?);
            proto_paths.push(path);
            Ok(())
        })?;

        // Compile the proto files
        let mut compiler = protox::Compiler::with_file_resolver(
            protox::file::DescriptorSetFileResolver::new(pool_raw),
        );
        compiler.include_source_info(true);
        compiler
            .open_files(proto_paths.iter().map(|p| p.to_path()))
            // rewrapping the error, so that source location is included in the error message.
            .map_err(|err| anyhow::anyhow!("{err:?}"))?;
        let descriptor = compiler.file_descriptor_set();
        pool.add_file_descriptor_set(descriptor.clone()).unwrap();

        // Check that the compiled proto files belong to the declared proto package.
        for f in &descriptor.file {
            let got = ProtoName::from(f.package());
            // Unwrap is ok, because descriptor file here has never an empty name.
            let want_prefix = ProtoPath::from(f.name()).parent().unwrap().to_name()?;
            if !got.starts_with(&want_prefix) {
                anyhow::bail!(
                    "{got} ({:?}) does not belong to package {want_prefix}",
                    f.name(),
                );
            }
        }

        // Check that the compiled proto messages support canonical encoding.
        canonical::check(&descriptor, &pool).context("canonical::check()")?;

        // Prepare the output directory.
        let output_dir = self
            .input_root
            .prepare_output_dir()
            .context("prepare_output_dir()")?;
        let output_path = output_dir.join("gen.rs");
        let descriptor_path = output_dir.join("gen.binpb");
        fs::write(&descriptor_path, &descriptor.encode_to_vec())?;
        println!("PROTOBUF_DESCRIPTOR={descriptor_path:?}");

        // Generate code out of compiled proto files.
        let mut output = RustModule::default();
        let mut config = prost_build::Config::new();
        config.prost_path(self.this_crate().join("prost").to_string());
        config.skip_protoc_run();
        for d in &self.dependencies {
            for f in &d.1.descriptor_proto.file {
                let proto_rel = ProtoName::from(f.package())
                    .relative_to(&d.1.proto_root)
                    .unwrap();
                let rust_abs = d.0.clone().join(proto_rel.to_rust_module());
                config.extern_path(format!(".{}", f.package()), rust_abs.to_string());
            }
        }
        let m = prost_build::Module::from_parts([""]);
        for f in &descriptor.file {
            let code = config
                .generate(vec![(m.clone(), f.clone())])
                .context("generation failed")?;
            output
                .sub(&ProtoName::from(f.package()).to_rust_module())
                .append(&code[&m]);
        }

        // Generate the reflection code.
        let package_root = self.proto_root.to_name().context("invalid proto_root")?;
        let output = output.sub(&package_root.to_rust_module());
        for proto_name in extract_message_names(&descriptor) {
            output.append(&self.reflect_impl(&proto_name)
                .with_context(||format!("reflect_impl({proto_name})"))?);
        }

        // Generate the descriptor.
        let rust_deps = self
            .dependencies
            .iter()
            .map(|d| format!("&{}::DESCRIPTOR", d.0))
            .collect::<Vec<_>>()
            .join(",");
        let this = self.this_crate().to_string();
        output.append(&format!("\
            pub static DESCRIPTOR : {this}::Lazy<{this}::Descriptor> = {this}::Lazy::new(|| {{\
                {this}::Descriptor::new(\"{package_root}\".into(), vec![{rust_deps}], &include_bytes!({descriptor_path:?})[..])\
            }});\
        "));

        // Save output.
        fs::write(output_path, output.format().context("output.format()")?)?;
        Ok(())
    }
}
