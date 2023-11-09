//! Checks whether messages in the given file descriptor set support canonical encoding.
use crate::syntax::extract_message_names;
use anyhow::Context as _;
use prost_reflect::prost_types;
use std::collections::HashSet;

#[derive(Default)]
struct Check(HashSet<String>);

impl Check {
    /// Checks if messages of type `message` support canonical encoding.
    fn check_message(&mut self, message: &prost_reflect::MessageDescriptor) -> anyhow::Result<()> {
        if self.0.contains(message.full_name()) {
            return Ok(());
        }
        self.0.insert(message.full_name().to_string());
        for field in message.fields() {
            self.check_field(&field)
                .with_context(|| field.name().to_string())?;
        }
        Ok(())
    }

    /// Checks if field `field` supports canonical encoding.
    fn check_field(&mut self, field: &prost_reflect::FieldDescriptor) -> anyhow::Result<()> {
        if field.is_map() {
            anyhow::bail!("maps unsupported");
        }
        if !field.is_list() && !field.supports_presence() {
            anyhow::bail!("non-repeated, non-oneof fields have to be marked as optional");
        }
        if let prost_reflect::Kind::Message(message) = &field.kind() {
            self.check_message(message)
                .with_context(|| message.name().to_string())?;
        }
        Ok(())
    }
}

/// Checks whether messages in the given file descriptor set support canonical encoding.
/// pool should contain all transitive dependencies of files in descriptor.
pub(crate) fn check(
    descriptor: &prost_types::FileDescriptorSet,
    pool: &prost_reflect::DescriptorPool,
) -> anyhow::Result<()> {
    for file in &descriptor.file {
        if file.syntax() != "proto3" {
            anyhow::bail!("{}: only proto3 syntax is supported", file.name());
        }
    }
    let mut check = Check::default();
    for msg_name in extract_message_names(descriptor) {
        let message_name = msg_name.to_string();
        let message = pool
            .get_message_by_name(&message_name)
            .with_context(|| format!("{msg_name} not found in pool"))?;
        check.check_message(&message).context(message_name)?;
    }
    Ok(())
}
