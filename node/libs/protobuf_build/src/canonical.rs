use anyhow::Context as _;
use std::collections::HashSet;
use super::*;

#[derive(Default)]
struct Check(HashSet<String>);

impl Check {
    /// Checks if messages of type `m` support canonical encoding.
    fn message(&mut self, m: &prost_reflect::MessageDescriptor) -> anyhow::Result<()> { 
        if self.0.contains(m.full_name()) {
            return Ok(());
        }
        self.0.insert(m.full_name().to_string());
        for f in m.fields() {
            self.field(&f).with_context(|| f.name().to_string())?;
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
        if let prost_reflect::Kind::Message(msg) = &f.kind() {
            self.message(msg).with_context(||msg.name().to_string())?;
        }
        Ok(())
    } 
}

pub fn check(descriptor: &prost_types::FileDescriptorSet, pool: &prost_reflect::DescriptorPool) -> anyhow::Result<()> {
    for f in &descriptor.file {
        if f.syntax() != "proto3" {
            anyhow::bail!("only proto3 syntax is supported");
        }
    }
    let mut c = Check::default();
    for msg_name in extract_message_names(descriptor) {
        let msg_name = msg_name.to_string();
        let msg = pool.get_message_by_name(&msg_name).with_context(||format!("{msg_name} not found in pool"))?;
        c.message(&msg).with_context(||msg_name)?;
    }
    Ok(())
}
