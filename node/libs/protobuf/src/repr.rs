//! `ProtoRepr` utilities.

use crate::build::prost_reflect::ReflectMessage;
use anyhow::Context as _;

/// Trait reverse to `zksync_protobuf::ProtoFmt` for cases where
/// you would like to specify a custom proto encoding for an externally defined type.
pub trait ProtoRepr: ReflectMessage + Default {
    /// The externally defined type associated with the proto Self.
    type Type;
    /// Converts proto Self to `Type`.
    fn read(&self) -> anyhow::Result<Self::Type>;
    /// Converts `Type` to proto Self.
    fn build(this: &Self::Type) -> Self;
}

/// Parses a required proto field.
pub fn read_required_repr<P: ProtoRepr>(field: &Option<P>) -> anyhow::Result<P::Type> {
    field.as_ref().context("missing field")?.read()
}
