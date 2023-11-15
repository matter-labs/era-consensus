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

/// Encodes a generated proto message to json for arbitrary ReflectMessage.
pub fn encode_json_proto<T: ReflectMessage>(x: &T) -> String {
    let mut s = serde_json::Serializer::pretty(vec![]);
    serialize_proto(x, &mut s).unwrap();
    String::from_utf8(s.into_inner()).unwrap()
}

/// Encodes a proto message to json for arbitrary ProtoFmt.
pub fn encode_json<T: ProtoFmt>(x: &T) -> String {
    encode_json_proto(&x.build())
}

/// Decodes a generated proto message from json for arbitrary ReflectMessage.
pub fn decode_json_proto<T: ReflectMessage + Default>(json: &str) -> anyhow::Result<T> {
    let mut d = serde_json::Deserializer::from_str(json);
    let p: T = deserialize_proto(&mut d)?;
    d.end()?;
    Ok(p)
}

/// Decodes a proto message from json for arbitrary ProtoFmt.
pub fn decode_json<T: ProtoFmt>(json: &str) -> anyhow::Result<T> {
    T::read(&decode_json_proto(json)?)
}
