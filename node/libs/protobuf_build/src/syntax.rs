//! Utilities for handling strings belonging to various namespaces.
use super::ident;
use anyhow::Context as _;
use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
};

/// Path relative to $CARGO_MANIFEST_DIR.
#[derive(Clone, PartialEq, Eq)]
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
#[derive(Clone, PartialEq, Eq)]
pub struct ProtoPath(PathBuf);

impl From<&str> for ProtoPath {
    fn from(s: &str) -> Self {
        Self(PathBuf::from(s))
    }
}

impl ProtoPath {
    /// Converts a proto module path to proto package name by replacing all "/" with ".".
    pub(super) fn to_name(&self) -> anyhow::Result<ProtoName> {
        let mut parts = vec![];
        for p in self.0.iter() {
            parts.push(p.to_str().context("invalid proto path")?.to_string());
        }
        Ok(ProtoName(parts))
    }

    /// Returns path to the parent directory.
    pub(super) fn parent(&self) -> Option<Self> {
        self.0.parent().map(|p| Self(p.into()))
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

    /// Converts ProtoPath to Path.
    pub(super) fn to_path(&self) -> &Path {
        &self.0
    }
}

impl fmt::Display for ProtoPath {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        self.0.display().fmt(fmt)
    }
}

type Part = String;

/// A rust module/type name that the generated code is available at. Although
/// generated code is location agnostic (it can be embedded in an arbitrary module within the crate),
/// you need to manually (in the Config) specify the rust modules containing the generated code
/// of the dependencies, so that it can be referenced from the newly generated code.
#[derive(Clone, PartialEq, Eq)]
pub struct RustName(Vec<Part>);

impl RustName {
    /// Concatenates 2 rust names.
    pub fn join(mut self, suffix: impl Into<Self>) -> Self {
        self.0.extend(suffix.into().0);
        self
    }
}

impl fmt::Display for RustName {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.write_str(&self.0.join("::"))
    }
}

impl From<prost_build::Module> for RustName {
    fn from(s: prost_build::Module) -> Self {
        Self(s.parts().map(Part::from).collect())
    }
}

impl From<&str> for RustName {
    fn from(s: &str) -> Self {
        Self(s.split("::").map(Part::from).collect())
    }
}

/// A rust module representation.
/// It is used to collect the generated protobuf code.
#[derive(Default)]
pub(super) struct RustModule {
    /// Nested modules which transitively contain the generated code.
    modules: BTreeMap<Part, RustModule>,
    /// Code of the module.
    code: String,
}

impl RustModule {
    /// Returns a reference to a given submodule.
    pub(crate) fn sub(&mut self, path: &RustName) -> &mut Self {
        let mut m = self;
        for part in &path.0 {
            m = m.modules.entry(part.into()).or_default();
        }
        m
    }

    /// Appends code to the module.
    pub(crate) fn append(&mut self, code: &str) {
        self.code += code;
    }

    fn collect(&self) -> String {
        let mut entries = vec![self.code.clone()];
        entries.extend(
            self.modules
                .iter()
                .map(|(name, m)| format!("pub mod {name} {{ {} }}\n", m.collect())),
        );
        entries.join("")
    }

    /// Collects the code of the module and formats it.
    pub(crate) fn format(&self) -> anyhow::Result<String> {
        Ok(prettyplease::unparse(
            &syn::parse_str(&self.collect()).context("syn::parse_str()")?,
        ))
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
#[derive(Clone, PartialEq, Eq)]
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

    /// Converts proto package name to rust module name according to prost_build rules.
    pub fn to_rust_module(&self) -> RustName {
        RustName(self.0.iter().map(|s| ident::to_snake(s)).collect())
    }

    /// Converts proto message name to rust type name according to prost_build rules.
    pub fn to_rust_type(&self) -> RustName {
        let mut rust = self.to_rust_module();
        let n = rust.0.len();
        rust.0[n - 1] = ident::to_upper_camel(&self.0[n - 1]);
        rust
    }
}

impl fmt::Display for ProtoName {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
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
    fn collect(out: &mut Vec<ProtoName>, prefix: &ProtoName, m: &prost_types::DescriptorProto) {
        let mut name = prefix.clone();
        name.0.push(m.name().to_string());
        for m in &m.nested_type {
            collect(out, &name, m);
        }
        out.push(name);
    }
    let mut res = vec![];
    for f in &descriptor.file {
        let name = ProtoName::from(f.package());
        for m in &f.message_type {
            collect(&mut res, &name, m);
        }
    }
    res
}
