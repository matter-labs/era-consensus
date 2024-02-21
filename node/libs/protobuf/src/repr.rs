//! Trait for defining proto conversion for external types.

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

/// Encodes a proto message.
/// Currently it outputs a canonical encoding, but `decode` accepts
/// non-canonical encoding as well.
pub fn encode<P: ProtoRepr>(msg: &P::Type) -> Vec<u8> {
    let msg = P::build(msg);
    super::canonical_raw(&msg.encode_to_vec(), &msg.descriptor()).unwrap()
}

/// Decodes a proto message.
pub fn decode<P: ProtoRepr>(bytes: &[u8]) -> anyhow::Result<P::Type> {
    P::read(&P::decode(bytes)?)
}
