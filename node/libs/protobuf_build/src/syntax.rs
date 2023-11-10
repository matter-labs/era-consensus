//! Utilities for handling strings belonging to various namespaces.

use super::ident;
use anyhow::Context as _;
use protox::prost_reflect::prost_types;
use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
    str::FromStr,
};

/// Path relative to `$CARGO_MANIFEST_DIR`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputPath(PathBuf);

impl From<&str> for InputPath {
    fn from(s: &str) -> Self {
        Self(PathBuf::from(s))
    }
}

impl InputPath {
    /// Converts the relative input path to str
    pub(super) fn to_str(&self) -> &str {
        self.0.to_str().unwrap()
    }

    /// Converts the relative input path to an absolute path in the local file system
    /// (under $CARGO_MANIFEST_DIR).
    pub(super) fn abs(&self) -> anyhow::Result<PathBuf> {
        Ok(PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?)
            .canonicalize()?
            .join(&self.0))
    }

    /// Output directory path derived from the input path by replacing $CARGO_MANIFEST_DIR with $OUT_DIR.
    /// Re-constructs the derived output directory, as a side-effect.
    pub(super) fn prepare_output_dir(&self) -> anyhow::Result<PathBuf> {
        let output = PathBuf::from(std::env::var("OUT_DIR")?)
            .canonicalize()?
            .join(&self.0);
        let _ = std::fs::remove_dir_all(&output);
        std::fs::create_dir_all(&output)?;
        Ok(output)
    }
}

/// Absolute path of the proto file used for importing other proto files.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProtoPath(PathBuf);

impl From<&str> for ProtoPath {
    fn from(s: &str) -> Self {
        Self(PathBuf::from(s))
    }
}

impl ProtoPath {
    /// Converts a proto module path to proto package name by replacing all "/" with ".".
    pub(super) fn to_name(&self) -> anyhow::Result<ProtoName> {
        let parts = self
            .0
            .iter()
            .map(|part| Ok(part.to_str().context("non-UTF8 proto path")?.to_string()));
        let parts = parts.collect::<anyhow::Result<_>>()?;
        Ok(ProtoName(parts))
    }

    /// Returns path to the parent directory.
    pub(super) fn parent(&self) -> Option<Self> {
        self.0.parent().map(|path| Self(path.to_owned()))
    }

    /// Derives a proto path from an input path by replacing the $CARGO_MANIFEST_DIR/<input_root> with <proto_root>.
    pub(super) fn from_input_path(
        path: &Path,
        input_root: &InputPath,
        proto_root: &ProtoPath,
    ) -> anyhow::Result<ProtoPath> {
        Ok(ProtoPath(
            proto_root
                .0
                .join(path.strip_prefix(&input_root.abs()?).unwrap()),
        ))
    }
}

impl AsRef<Path> for ProtoPath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl fmt::Display for ProtoPath {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.display().fmt(fmt)
    }
}

/// A rust module/type name that the generated code is available at. Although
/// generated code is location agnostic (it can be embedded in an arbitrary module within the crate),
/// you need to manually (in the Config) specify the rust modules containing the generated code
/// of the dependencies, so that it can be referenced from the newly generated code.
#[derive(Clone)]
pub struct RustName(syn::Path);

impl RustName {
    /// Creates a name consisting of a single identifier.
    pub(crate) fn ident(ident: &str) -> Self {
        let ident: syn::Ident = syn::parse_str(ident).expect("Invalid identifier");
        Self(syn::Path::from(ident))
    }

    /// Returns the crate name for this name (i.e., its first segment).
    pub(crate) fn crate_name(&self) -> Option<String> {
        Some(self.0.segments.first()?.ident.to_string())
    }

    /// Concatenates 2 rust names.
    pub fn join(mut self, suffix: Self) -> Self {
        self.0.segments.extend(suffix.0.segments);
        self
    }
}

impl FromStr for RustName {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let path: syn::Path =
            syn::parse_str(s).with_context(|| format!("failed parsing path `{s}`"))?;
        for segment in &path.segments {
            anyhow::ensure!(
                segment.arguments.is_empty(),
                "path must be a plain `::`-delimited path (no path arguments)"
            );
        }
        Ok(Self(path))
    }
}

impl fmt::Display for RustName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = &self.0;
        fmt::Display::fmt(&quote::quote!(#path), formatter)
    }
}

impl quote::ToTokens for RustName {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.0.to_tokens(tokens)
    }
}

/// A rust module representation.
/// It is used to collect the generated protobuf code.
pub(super) struct RustModule {
    /// Nested modules which transitively contain the generated code.
    submodules: BTreeMap<syn::Ident, RustModule>,
    /// Code of the module.
    code: syn::File,
}

impl Default for RustModule {
    fn default() -> Self {
        Self {
            submodules: BTreeMap::new(),
            code: syn::File {
                shebang: None,
                attrs: vec![],
                items: vec![],
            },
        }
    }
}

impl RustModule {
    /// Returns a reference to a given submodule.
    pub(crate) fn submodule(&mut self, path: &RustName) -> &mut Self {
        let mut module = self;
        for part in &path.0.segments {
            module = module.submodules.entry(part.ident.clone()).or_default();
        }
        module
    }

    /// Converts this module into a submodule at the specified path.
    pub(crate) fn into_submodule(self, path: &RustName) -> Self {
        let mut module = self;
        for part in &path.0.segments {
            module = module.submodules.remove(&part.ident).unwrap_or_default();
        }
        module
    }

    /// Extends this module with the specified code.
    pub(crate) fn extend(&mut self, code: syn::File) {
        if self.code.items.is_empty() {
            self.code = code;
        } else {
            self.code.items.extend(code.items);
        }
    }

    /// Appends the specified item to this module.
    pub(crate) fn append_item(&mut self, item: syn::Item) {
        self.code.items.push(item);
    }

    fn collect(mut self) -> syn::File {
        for (name, submodule) in self.submodules {
            let submodule = submodule.collect();
            self.code.items.push(syn::parse_quote! {
                pub mod #name {
                    #submodule
                }
            });
        }
        self.code
    }

    /// Collects the code of the module and formats it.
    pub(crate) fn format(self) -> String {
        prettyplease::unparse(&self.collect())
    }
}

/// In addition to input paths and proto paths, there are also proto names
/// which are used to reference message types from different proto files.
/// Theoretically proto names can be totally independent from the proto paths (i.e. you can have
/// "my.favorite.package" proto package defined in "totally/unrelated/file/name.proto" file, but we
/// recommend the following naming convention: proto package "a.b.c" should be defined either:
/// a) in a single file "a/b/c.proto", or
/// b) in a collection of files under "a/b/c/" directory
/// Option b) is useful for defining large packages, because there is no equivalent of "pub use" in proto syntax.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtoName(Vec<String>);

impl ProtoName {
    /// Checks if package path starts with the given prefix.
    pub fn starts_with(&self, prefix: &Self) -> bool {
        self.0.len() >= prefix.0.len() && self.0[0..prefix.0.len()] == prefix.0
    }

    /// Strips a given prefix from the name.
    pub fn relative_to(&self, prefix: &Self) -> anyhow::Result<Self> {
        if !self.starts_with(prefix) {
            anyhow::bail!("{self} does not contain {prefix}");
        }
        Ok(Self(self.0[prefix.0.len()..].to_vec()))
    }

    /// Converts proto package name to rust module name according to `prost_build` rules.
    pub fn to_rust_module(&self) -> anyhow::Result<RustName> {
        let segments = self.0.iter().map(|segment| {
            let ident: syn::Ident = syn::parse_str(&ident::to_snake(segment))
                .with_context(|| format!("Invalid identifier `{segment}`"))?;
            Ok(syn::PathSegment::from(ident))
        });
        Ok(RustName(syn::Path {
            leading_colon: None,
            segments: segments.collect::<anyhow::Result<_>>()?,
        }))
    }

    /// Converts proto message name to rust type name according to `prost_build` rules.
    pub fn to_rust_type(&self) -> anyhow::Result<RustName> {
        let mut rust = self.to_rust_module()?;
        let type_name = self.0.last().expect("`ProtoName` cannot be empty");
        let type_name = ident::to_upper_camel(type_name);
        let last_segment = rust.0.segments.last_mut().unwrap();
        *last_segment = syn::parse_str(&type_name)
            .with_context(|| format!("Invalid identifier `{type_name}`"))?;
        Ok(rust)
    }
}

impl fmt::Display for ProtoName {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.write_str(&self.0.join("."))
    }
}

impl From<&str> for ProtoName {
    fn from(s: &str) -> Self {
        Self(s.split('.').map(String::from).collect())
    }
}

impl From<&Path> for ProtoName {
    fn from(p: &Path) -> Self {
        Self(p.iter().map(|c| c.to_str().unwrap().to_string()).collect())
    }
}

/// Extracts names of proto messages defined in the descriptor.
pub(super) fn extract_message_names(descriptor: &prost_types::FileDescriptorSet) -> Vec<ProtoName> {
    fn collect(
        out: &mut Vec<ProtoName>,
        prefix: &ProtoName,
        message: &prost_types::DescriptorProto,
    ) {
        let mut name = prefix.clone();
        name.0.push(message.name().to_string());
        for nested_message in &message.nested_type {
            collect(out, &name, nested_message);
        }
        out.push(name);
    }

    let mut names = vec![];
    for file in &descriptor.file {
        let name = ProtoName::from(file.package());
        for message in &file.message_type {
            collect(&mut names, &name, message);
        }
    }
    names
}
