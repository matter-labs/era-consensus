use anyhow::Context as _;
use std::collections::HashSet;

#[derive(Default)]
struct Check(HashSet<String>);

impl Check {
    /// Checks if messages of type `m` support canonical encoding.
    fn message(&mut self, m: &prost_reflect::MessageDescriptor, check_nested: bool) -> anyhow::Result<()> { 
        if self.0.contains(m.full_name()) {
            return Ok(());
        }
        self.0.insert(m.full_name().to_string());
        for f in m.fields() {
            self.field(&f).with_context(|| f.name().to_string())?;
        }
        if check_nested {
            for m in m.child_messages() {
                self.message(&m,check_nested).with_context(||m.name().to_string())?;
            }
        }
        Ok(())
    }

    /// Checks if field `f` supports canonical encoding.
    fn field(&mut self, f: &prost_reflect::FieldDescriptor) -> anyhow::Result<()> {
        if f.is_map() {
            anyhow::bail!("maps unsupported");
        }
        if !f.is_list() && !f.supports_presence() {
            anyhow::bail!("non-repeated, non-oneof fields have to be marked as optional");
        }
        if let prost_reflect::Kind::Message(msg) = f.kind() {
            self.message(&msg,false).with_context(||msg.name().to_string())?;
        }
        Ok(())
    }

    /// Checks if message types in file `f` support canonical encoding.
    fn file(&mut self, f: &prost_reflect::FileDescriptor) -> anyhow::Result<()> {
        if f.syntax() != prost_reflect::Syntax::Proto3 {
            anyhow::bail!("only proto3 syntax is supported");
        }
        for m in f.messages() {
            self.message(&m,true).with_context(|| m.name().to_string())?;
        }
        Ok(())
    }
}

pub fn check(descriptor: &prost_types::FileDescriptorSet, pool: &prost_reflect::DescriptorPool) -> anyhow::Result<()> {
    let mut c = Check::default();
    for f in &descriptor.file {
        c.file(f).with_context(||f.name())?;
    }
    Ok(())
}
