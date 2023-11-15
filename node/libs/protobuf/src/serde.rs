//! Implementation of serde traits for structs implementing ProtoFmt.
//! This serde implementation is compatible with protobuf json mapping,
//! therefore it is suitable for version control.
//! WARNING: Currently this serde implementation uses reflection,
//! so it is not very efficient.
use crate::ProtoFmt;
use prost::Message as _;
use prost_reflect::ReflectMessage;

/// Implementation of serde::Serialize for arbitrary ReflectMessage.
pub fn serialize_proto<T: ReflectMessage, S: serde::Serializer>(
    x: &T,
    s: S,
) -> Result<S::Ok, S::Error> {
    let opts = prost_reflect::SerializeOptions::new();
    x.transcode_to_dynamic().serialize_with_options(s, &opts)
}

/// Implementation of serde::Serialize for arbitrary ProtoFmt.
pub fn serialize<T: ProtoFmt, S: serde::Serializer>(x: &T, s: S) -> Result<S::Ok, S::Error> {
    serialize_proto(&x.build(), s)
}

/// Implementation of serde::Deserialize for arbitrary ReflectMessage.
pub fn deserialize_proto<'de, T: ReflectMessage + Default, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<T, D::Error> {
    let mut p = T::default();
    let msg = prost_reflect::DynamicMessage::deserialize(p.descriptor(), d)?;
    p.merge(msg.encode_to_vec().as_slice()).unwrap();
    Ok(p)
}

/// Implementation of serde::Deserialize for arbitrary ProtoFmt.
pub fn deserialize<'de, T: ProtoFmt, D: serde::Deserializer<'de>>(d: D) -> Result<T, D::Error> {
    T::read(&deserialize_proto(d)?).map_err(serde::de::Error::custom)
}
