use std::collections::BTreeMap;

type Part = String;

#[derive(Clone,PartialEq,Eq)]
pub struct Name(Vec<Part>);

impl Name {
    fn add(mut self, suffix: impl Into<Self>) -> Self {
        self.0.extend(suffix.into().0.into_iter());
        self
    }

    fn to_string(&self) -> String { self.0.join("::") }
}

impl From<prost_build::Module> for Name {
    fn from(s:prost_build::Module) -> Self { Self(s.parts().map(Part::from).collect()) }
}

impl From<&str> for Name {
    fn from(s:&str) -> Self { Self(s.split("::").map(Part::from).collect()) }
}

/// A rust module representation.
/// It is used to collect the generated protobuf code.
#[derive(Default)]
pub(super) struct Module {
    /// Nested modules which transitively contain the generated code.
    modules: BTreeMap<Part, Module>,
    /// Code of the module.
    code: String,
}

impl Module {
    pub fn sub(&self, path: &Name) -> &mut Self {
        let mut m = self;
        for part in &path.0 {
            m = m.modules.entry(part.into()).or_default();
        }
        m
    }

    /// Appends code to the module.
    pub fn append(&mut self, code: &str) {
        self.code += code;
    }

    /// Collects the code of the module.
    pub fn format(&self) -> anyhow::Result<String> {
        let mut entries = vec![self.code.clone()];
        entries.extend(
            self.modules.iter().map(|(name, m)| format!("pub mod {name} {{ {} }}\n", m.collect())),
        );
        prettyplease::unparse(syn::parse_str(&entries.join("")).context("syn::parse_str()")?)
    }
}
