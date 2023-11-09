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
// Imports accessed from the generated code.
pub use self::syntax::*;
use anyhow::Context as _;
pub use once_cell::sync::Lazy;
pub use prost;
use prost::Message as _;
pub use prost_reflect;
use std::{fs, path::Path, sync::Mutex};

mod canonical;
mod ident;
mod syntax;

/// Traverses all the files in a directory recursively.
fn traverse_files(
    path: &Path,
    action: &mut impl FnMut(&Path) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    if !path.is_dir() {
        action(path).with_context(|| path.display().to_string())?;
        return Ok(());
    }
    for entry in fs::read_dir(path)? {
        traverse_files(&entry?.path(), action)?;
    }
    Ok(())
}

/// Protobuf descriptor + info about the mapping to rust code.
#[derive(Debug)]
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
            descriptor_proto: prost_types::FileDescriptorSet::decode(descriptor_bytes).unwrap(),
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
        for dependency in &self.dependencies {
            dependency.load(pool)?;
        }
        pool.add_file_descriptor_set(self.descriptor_proto.clone())?;
        Ok(())
    }

    /// Loads the descriptor to the global pool and returns a copy of the global pool.
    pub fn get_message_by_name(&self, name: &str) -> Option<prost_reflect::MessageDescriptor> {
        /// Global descriptor pool.
        static POOL: Lazy<Mutex<prost_reflect::DescriptorPool>> = Lazy::new(Mutex::default);
        let pool = &mut POOL.lock().unwrap();
        self.load(pool).unwrap();
        pool.get_message_by_name(name)
    }
}

/// Expands to a descriptor declaration.
#[macro_export]
macro_rules! declare_descriptor {
    ($package_root:expr, $descriptor_path:expr, $($rust_deps:path),*) => {
        pub static DESCRIPTOR: $crate::Lazy<$crate::Descriptor> = $crate::Lazy::new(|| {
            $crate::Descriptor::new(
                $package_root.into(),
                ::std::vec![$({ use $rust_deps as dep; &dep::DESCRIPTOR }),*],
                include_bytes!($descriptor_path),
            )
        });
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
        self.protobuf_crate.clone().join(RustName::ident("build"))
    }

    /// Generates implementation of `prost_reflect::ReflectMessage` for a rust type generated
    /// from a message of the given `proto_name`.
    fn reflect_impl(&self, proto_name: &ProtoName) -> anyhow::Result<syn::ItemImpl> {
        let rust_name = proto_name
            .relative_to(&self.proto_root.to_name().context("invalid proto_root")?)
            .unwrap()
            .to_rust_type()?;
        let proto_name = proto_name.to_string();
        let this = self.this_crate();
        Ok(syn::parse_quote! {
            impl #this::prost_reflect::ReflectMessage for #rust_name {
                fn descriptor(&self) -> #this::prost_reflect::MessageDescriptor {
                    static INIT: #this::Lazy<#this::prost_reflect::MessageDescriptor> = #this::Lazy::new(|| {
                        DESCRIPTOR.get_message_by_name(#proto_name).unwrap()
                    });
                    INIT.clone()
                }
            }
        })
    }

    /// Generates rust code from the proto files according to the config.
    pub fn generate(&self) -> anyhow::Result<()> {
        if !self.input_root.abs()?.is_dir() {
            anyhow::bail!("input_root should be a directory");
        }
        println!("cargo:rerun-if-changed={}", self.input_root.to_str());

        // Load dependencies.
        let mut pool = prost_reflect::DescriptorPool::new();
        for (root_path, descriptor) in &self.dependencies {
            descriptor
                .load(&mut pool)
                .with_context(|| format!("failed to load dependency `{root_path}`"))?;
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
            let path = ProtoPath::from_input_path(path, &self.input_root, &self.proto_root)
                .context("ProtoPath::from_input_path()")?;
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
            .open_files(proto_paths)
            // rewrapping the error, so that source location is included in the error message.
            .map_err(|err| anyhow::anyhow!("{err:?}"))?;
        let descriptor = compiler.file_descriptor_set();
        // Unwrap is ok, because we add a descriptor from a successful compilation.
        pool.add_file_descriptor_set(descriptor.clone()).unwrap();

        // Check that the compiled proto files belong to the declared proto package.
        for file in &descriptor.file {
            let got = ProtoName::from(file.package());
            // Unwrap is ok, because descriptor file here has never an empty name.
            let want_prefix = ProtoPath::from(file.name()).parent().unwrap().to_name()?;
            anyhow::ensure!(
                got.starts_with(&want_prefix),
                "{got} ({:?}) does not belong to package {want_prefix}",
                file.name()
            );
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
        let prost_path = self.this_crate().join(RustName::ident("prost"));
        config.prost_path(prost_path.to_string());
        config.skip_protoc_run();
        for (root_path, descriptor) in &self.dependencies {
            for file in &descriptor.descriptor_proto.file {
                let proto_rel = ProtoName::from(file.package())
                    .relative_to(&descriptor.proto_root)
                    .unwrap();
                let rust_path = root_path.clone().join(proto_rel.to_rust_module()?);
                config.extern_path(format!(".{}", file.package()), rust_path.to_string());
            }
        }
        let module = prost_build::Module::from_parts([""]);
        for file in &descriptor.file {
            let code = config
                .generate(vec![(module.clone(), file.clone())])
                .context("generation failed")?;
            let code = &code[&module];
            let code = syn::parse_str(code).with_context(|| {
                format!("prost_build generated invalid code for {}", file.name())
            })?;
            output
                .submodule(&ProtoName::from(file.package()).to_rust_module()?)
                .extend(code);
        }

        // Generate the reflection code.
        let package_root = self.proto_root.to_name().context("invalid proto_root")?;
        let mut output = output.into_submodule(&package_root.to_rust_module()?);
        for proto_name in extract_message_names(&descriptor) {
            let impl_item = self
                .reflect_impl(&proto_name)
                .with_context(|| format!("reflect_impl({proto_name})"))?;
            output.append_item(impl_item.into());
        }

        // Generate the descriptor.
        let root_paths_for_deps = self.dependencies.iter().map(|(root_path, _)| root_path);
        let this = self.this_crate();
        let package_root = package_root.to_string();
        let descriptor_path = descriptor_path.display().to_string();
        output.append_item(syn::parse_quote! {
            #this::declare_descriptor!(#package_root, #descriptor_path, #(#root_paths_for_deps),*);
        });

        // Save output.
        fs::write(&output_path, output.format()).with_context(|| {
            format!(
                "failed writing generated code to `{}`",
                output_path.display()
            )
        })?;
        Ok(())
    }
}
