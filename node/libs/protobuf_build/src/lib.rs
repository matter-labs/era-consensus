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

pub use self::syntax::*;
use anyhow::Context as _;
use prost_reflect::{prost::Message as _, prost_types};
use protox::file::File;
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

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

/// Manifest of a Protobuf compilation target containing information about the compilation process.
#[derive(Debug)]
struct Manifest {
    /// Root proto package that all proto files in this descriptor belong to.
    proto_root: ProtoPath,
    /// Absolute path to the descriptor.
    descriptor_path: PathBuf,
    /// Tuples of `proto_root` and absolute paths to the corresponding descriptor for all dependencies
    /// including transitive ones.
    dependencies: Vec<(ProtoPath, PathBuf)>,
}

impl Manifest {
    /// Loads manifest from the environment variable.
    fn from_env(rust_root: &RustName) -> anyhow::Result<Self> {
        let crate_name = rust_root.crate_name().context("empty `rust_root`")?;
        let env_name = format!("DEP_{}_PROTO_MANIFEST", crate_name.to_uppercase());
        let manifest = env::var(env_name)
            .with_context(|| format!("failed reading path to `{crate_name}` Protobuf manifest"))?;

        let mut manifest_parts = manifest.split(':');
        // ^ ':' is used as a separator since it cannot be present in paths.
        let proto_root = manifest_parts.next().context("missing `proto_root`")?;
        let proto_root = ProtoPath::from(proto_root);
        let descriptor_path = manifest_parts
            .next()
            .context("missing `descriptor_path`")?
            .into();

        let mut dependencies = vec![];
        while let Some(proto_root) = manifest_parts.next() {
            let proto_root = ProtoPath::from(proto_root);
            let descriptor_path = manifest_parts
                .next()
                .context("missing `descriptor_path`")?
                .into();
            dependencies.push((proto_root, descriptor_path));
        }

        Ok(Self {
            proto_root,
            descriptor_path,
            dependencies,
        })
    }

    /// Prints this manifest to an environment variable so that it's available to dependencies.
    fn print(&self) {
        use std::fmt::Write as _;

        let Self {
            proto_root,
            descriptor_path,
            dependencies,
        } = self;
        let dependencies = dependencies
            .iter()
            .fold(String::new(), |mut acc, (root, desc_path)| {
                write!(&mut acc, ":{root}:{}", desc_path.display()).unwrap();
                acc
            });
        println!(
            "cargo:manifest={proto_root}:{}{dependencies}",
            descriptor_path.display()
        );
    }
}

/// Code generation config. Use it in build scripts.
pub struct Config {
    /// Input directory relative to $CARGO_MANIFEST_DIR with the proto files to be compiled.
    pub input_root: InputPath,
    /// Implicit prefix that should be prepended to proto paths of the proto files in the input directory.
    pub proto_root: ProtoPath,
    /// Descriptors of the direct dependencies and the rust absolute paths under which they will be available from the generated code.
    /// Each dependency must be a direct dependency of the built crate. OTOH, it doesn't need to be a build dependency.
    pub dependencies: Vec<RustName>,
    /// Rust absolute path under which the protobuf crate will be available from the generated code.
    pub protobuf_crate: RustName,
    /// Can generated Protobuf messages be included as a dependency for other crates (i.e., be mentioned
    /// in `dependencies`)? Only one public target can be generated per build script.
    pub is_public: bool,
}

impl Config {
    /// Generates implementation of `prost_reflect::ReflectMessage` for a rust type generated
    /// from a message of the given `proto_name`.
    fn reflect_impl(&self, proto_name: &ProtoName) -> anyhow::Result<syn::Item> {
        let rust_name = proto_name
            .relative_to(&self.proto_root.to_name().context("invalid proto_root")?)
            .unwrap()
            .to_rust_type()?;
        let proto_name = proto_name.to_string();
        let protobuf_crate = &self.protobuf_crate;
        Ok(
            syn::parse_quote! { #protobuf_crate::build::impl_reflect_message!(#rust_name, &DESCRIPTOR, #proto_name); },
        )
    }

    /// Validates this configuration.
    fn validate(&self) -> anyhow::Result<()> {
        /// Flag set to `true` if a public compilation target was encountered in a build script.
        static HAS_PUBLIC_TARGET: AtomicBool = AtomicBool::new(false);

        if !self.input_root.abs()?.is_dir() {
            anyhow::bail!("input_root should be a directory");
        }
        if self.is_public {
            anyhow::ensure!(
                !HAS_PUBLIC_TARGET.fetch_or(true, Ordering::SeqCst),
                "Only one compilation target with `is_public: true` may be specified per build script"
            );

            let crate_name = env::var("CARGO_PKG_NAME")
                .context("missing $CARGO_PKG_NAME env variable")?
                .replace('-', "_");
            let expected_name = format!("{crate_name}_proto");
            let links = env::var("CARGO_MANIFEST_LINKS").ok();
            anyhow::ensure!(
                links.as_ref() == Some(&expected_name),
                "You must specify links = \"{expected_name}\" in the [package] section \
                 of the built package manifest (currently set to {links:?})"
            );
        }
        Ok(())
    }

    /// Generates rust code from the proto files according to the config.
    pub fn generate(self) -> anyhow::Result<()> {
        self.validate()?;
        println!("cargo:rerun-if-changed={}", self.input_root.to_str());

        // Load dependencies.
        let dependency_manifests = self.dependencies.iter().map(Manifest::from_env);
        let dependency_manifests: Vec<Manifest> =
            dependency_manifests.collect::<anyhow::Result<_>>()?;
        let direct_dependency_descriptor_paths: HashSet<_> = dependency_manifests
            .iter()
            .map(|manifest| &manifest.descriptor_path)
            .collect();

        let all_dependencies = dependency_manifests.iter().flat_map(|manifest| {
            manifest
                .dependencies
                .iter()
                .map(|(root, path)| (root, path))
                // ^ Converts a reference to a tuple to a tuple of references
                .chain([(&manifest.proto_root, &manifest.descriptor_path)])
        });

        let mut pool = prost_reflect::DescriptorPool::new();
        let mut direct_dependency_descriptors = HashMap::with_capacity(self.dependencies.len());
        let mut loaded_descriptor_paths = HashSet::new();
        let mut dependencies = vec![];
        for (proto_root, descriptor_path) in all_dependencies {
            if !loaded_descriptor_paths.insert(descriptor_path) {
                // Do not load the same descriptor twice.
                continue;
            }
            dependencies.push((proto_root.clone(), descriptor_path.clone()));

            let descriptor = fs::read(descriptor_path).with_context(|| {
                format!(
                    "failed reading descriptor for `{proto_root}` from {}",
                    descriptor_path.display()
                )
            })?;
            let descriptor =
                prost_types::FileDescriptorSet::decode(&descriptor[..]).with_context(|| {
                    format!(
                        "failed decoding file descriptor set for `{proto_root}` from {}",
                        descriptor_path.display()
                    )
                })?;

            if direct_dependency_descriptor_paths.contains(descriptor_path) {
                direct_dependency_descriptors.insert(descriptor_path.clone(), descriptor.clone());
            }
            pool.add_file_descriptor_set(descriptor)?;
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

            let source = fs::read_to_string(path).context("fs::read()")?;
            let path = ProtoPath::from_input_path(path, &self.input_root, &self.proto_root)
                .context("ProtoPath::from_input_path()")?;
            let compiled = File::from_source(&path.to_string(), &source).map_err(
                // rewrapping the error, so that source location is included in the error message.
                |err| anyhow::anyhow!("{err:?}"),
            )?;
            pool_raw.file.push(compiled.into());
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
        fs::write(&descriptor_path, descriptor.encode_to_vec())?;

        if self.is_public {
            let manifest = Manifest {
                proto_root: self.proto_root.clone(),
                descriptor_path: descriptor_path.clone(),
                dependencies,
            };
            manifest.print();
        }
        println!("PROTOBUF_DESCRIPTOR={descriptor_path:?}");

        // Generate code out of compiled proto files.
        let mut output = RustModule::default();
        let mut config = prost_build::Config::new();
        let prost_path = self
            .protobuf_crate
            .clone()
            .join(RustName::ident("build"))
            .join(RustName::ident("prost"));
        config.prost_path(prost_path.to_string());
        config.skip_protoc_run();
        let mut extern_paths = HashMap::new();
        for (root_path, manifest) in self.dependencies.iter().zip(&dependency_manifests) {
            let descriptor = &direct_dependency_descriptors[&manifest.descriptor_path];
            // ^ Indexing is safe by construction.
            for file in &descriptor.file {
                let proto_rel = ProtoName::from(file.package())
                    .relative_to(&manifest.proto_root.to_name()?)
                    .unwrap();
                let rust_path = root_path.clone().join(proto_rel.to_rust_module()?);
                match extern_paths.insert(file.package(), rust_path.clone()) {
                    // If missing, add to config.
                    None => { config.extern_path(format!(".{}", file.package()), rust_path.to_string()); }
                    // If already present, make sure that it is the same.
                    Some(old) => anyhow::ensure!(rust_path==old, "ambiguous rust path for proto package {}",file.package()),
                }
            }
        }
        let module = prost_build::Module::from_parts([""]);
        for file in &descriptor.file {
            let code = config
                .generate(vec![(module.clone(), file.clone())])
                .context("generation failed")?;
            // It may happen that the proto package is empty,
            // in which case no code gets generated.
            let Some(code) = code.get(&module) else {
                continue;
            };
            let code = syn::parse_str(code).with_context(|| {
                format!("prost_build generated invalid code for {}", file.name())
            })?;
            output
                .submodule(&ProtoName::from(file.package()).to_rust_module()?)
                .extend(code);
        }

        let package_root = self.proto_root.to_name().context("invalid proto_root")?;
        let mut output = output.into_submodule(&package_root.to_rust_module()?);

        // Generate the descriptor.
        let root_paths_for_deps = self.dependencies.iter();
        let protobuf_crate = &self.protobuf_crate;
        let descriptor_path = descriptor_path.display().to_string();
        output.append_item(syn::parse_quote! {
            #protobuf_crate::build::declare_descriptor!(DESCRIPTOR => #descriptor_path, #(#root_paths_for_deps),*);
        });

        // Generate the reflection code.
        for proto_name in extract_message_names(&descriptor) {
            let item = self
                .reflect_impl(&proto_name)
                .with_context(|| format!("reflect_impl({proto_name})"))?;
            output.append_item(item);
        }

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
