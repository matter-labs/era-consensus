//! Implementation of serde traits for structs implementing ProtoFmt.
//! This serde implementation is compatible with protobuf json mapping,
//! therefore it is suitable for version control.
//! WARNING: Currently this serde implementation uses reflection,
//! so it is not very efficient.
use crate::ProtoFmt;
use prost::Message as _;
use prost_reflect::ReflectMessage;

/// ProtoFmt wrapper which implements serde Serialize/Deserialize.
#[derive(Debug, Clone)]
pub struct Serde<T>(pub T);

impl<T: ProtoFmt> serde::Serialize for Serde<T> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        serialize(&self.0, s)
    }
}

impl<'de, T: ProtoFmt> serde::Deserialize<'de> for Serde<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self(deserialize(d)?))
    }
}

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

/// Implementation of serde::Deserialize for arbitrary ReflectMessage denying unknown fields
pub fn deserialize_proto<'de, T: ReflectMessage + Default, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<T, D::Error> {
    deserialize_proto_with_options(d, true)
}

/// Implementation of serde::Deserialize for arbitrary ReflectMessage with deny_unknown_fields option
pub fn deserialize_proto_with_options<
    'de,
    T: ReflectMessage + Default,
    D: serde::Deserializer<'de>,
>(
    d: D,
    deny_unknown_fields: bool,
) -> Result<T, D::Error> {
    let mut p = T::default();
    let options = prost_reflect::DeserializeOptions::new().deny_unknown_fields(deny_unknown_fields);
    let msg = prost_reflect::DynamicMessage::deserialize_with_options(p.descriptor(), d, &options)?;
    p.merge(msg.encode_to_vec().as_slice()).unwrap();
    Ok(p)
}

/// Implementation of serde::Deserialize for arbitrary ProtoFmt.
pub fn deserialize<'de, T: ProtoFmt, D: serde::Deserializer<'de>>(d: D) -> Result<T, D::Error> {
    T::read(&deserialize_proto(d)?).map_err(serde::de::Error::custom)
}
