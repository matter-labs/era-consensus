//! Implementation of serde traits for structs implementing ProtoFmt.
//! This serde implementation is compatible with protobuf json mapping,
//! therefore it is suitable for version control.
//! WARNING: Currently this serde implementation uses reflection,
//! so it is not very efficient.
use crate::{ProtoFmt, ProtoRepr};
use prost::Message as _;
use prost_reflect::ReflectMessage;

/// Serialization options.
pub struct Serialize;

impl Serialize {
    /// Serializes ReflectMessage.
    pub fn proto<T: ReflectMessage, S: serde::Serializer>(
        &self,
        x: &T,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        let opts = prost_reflect::SerializeOptions::new();
        x.transcode_to_dynamic().serialize_with_options(s, &opts)
    }

    /// Serializes ProtoFmt.
    pub fn proto_fmt<T: ProtoFmt, S: serde::Serializer>(
        &self,
        x: &T,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        self.proto(&x.build(), s)
    }

    /// Serializes ProtoFmt to json.
    pub fn proto_fmt_to_json<T: ProtoFmt>(&self, x: &T) -> String {
        let mut s = serde_json::Serializer::pretty(vec![]);
        self.proto_fmt(x, &mut s).unwrap();
        String::from_utf8(s.into_inner()).unwrap()
    }

    /// Serializes ProtoFmt to yaml
    pub fn proto_fmt_to_yaml<T: ProtoFmt>(&self, x: &T) -> String {
        let mut s = serde_yaml::Serializer::new(vec![]);
        self.proto_fmt(x, &mut s).unwrap();
        String::from_utf8(s.into_inner().unwrap()).unwrap()
    }

    /// Serializes ProtoRepr.
    pub fn proto_repr<T: ProtoRepr, S: serde::Serializer>(
        &self,
        x: &T::Type,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        self.proto(&T::build(x), s)
    }

    /// Serializes ProtoRepr to json.
    pub fn proto_repr_to_json<T: ProtoRepr>(&self, x: &T::Type) -> String {
        let mut s = serde_json::Serializer::pretty(vec![]);
        self.proto_repr::<T, _>(x, &mut s).unwrap();
        String::from_utf8(s.into_inner()).unwrap()
    }

    /// Serializes ProtoRepr to yaml
    pub fn proto_repr_to_yaml<T: ProtoRepr>(&self, x: &T::Type) -> String {
        let mut s = serde_yaml::Serializer::new(vec![]);
        self.proto_repr::<T, _>(x, &mut s).unwrap();
        String::from_utf8(s.into_inner().unwrap()).unwrap()
    }
}

/// Deserialization options.
#[derive(Default)]
pub struct Deserialize {
    /// true => returns an error when an unknown field is found.
    /// false => silently ignores unknown fields.
    pub deny_unknown_fields: bool,
}

impl Deserialize {
    /// Implementation of serde::Deserialize for arbitrary ReflectMessage with deny_unknown_fields option
    pub fn proto<'de, T: ReflectMessage + Default, D: serde::Deserializer<'de>>(
        &self,
        d: D,
    ) -> Result<T, D::Error> {
        let mut p = T::default();
        let options =
            prost_reflect::DeserializeOptions::new().deny_unknown_fields(self.deny_unknown_fields);
        let msg =
            prost_reflect::DynamicMessage::deserialize_with_options(p.descriptor(), d, &options)?;
        p.merge(msg.encode_to_vec().as_slice()).unwrap();
        Ok(p)
    }

    /// Implementation of serde::Deserialize for arbitrary ProtoFmt.
    pub fn proto_fmt<'de, T: ProtoFmt, D: serde::Deserializer<'de>>(
        &self,
        d: D,
    ) -> Result<T, D::Error> {
        T::read(&self.proto(d)?).map_err(serde::de::Error::custom)
    }

    /// Deserializes ProtoFmt from json.
    pub fn proto_fmt_from_json<T: ProtoFmt>(&self, json: &str) -> anyhow::Result<T> {
        let mut d = serde_json::Deserializer::from_str(json);
        let p = self.proto_fmt(&mut d)?;
        d.end()?;
        Ok(p)
    }

    /// Deserializes ProtoFmt from yaml.
    pub fn proto_fmt_from_yaml<T: ProtoFmt>(&self, yaml: &str) -> anyhow::Result<T> {
        Ok(self.proto_fmt(serde_yaml::Deserializer::from_str(yaml))?)
    }

    /// Implementation of serde::Deserialize for arbitrary ProtoFmt.
    pub fn proto_repr<'de, T: ProtoRepr, D: serde::Deserializer<'de>>(
        &self,
        d: D,
    ) -> Result<T::Type, D::Error> {
        self.proto::<T, D>(d)?
            .read()
            .map_err(serde::de::Error::custom)
    }

    /// Deserializes ProtoRepr from json.
    pub fn proto_repr_from_json<T: ProtoRepr>(&self, json: &str) -> anyhow::Result<T::Type> {
        let mut d = serde_json::Deserializer::from_str(json);
        let p = self.proto_repr::<T, _>(&mut d)?;
        d.end()?;
        Ok(p)
    }

    /// Deserializes ProtoRepr from yaml.
    pub fn proto_repr_from_yaml<T: ProtoRepr>(&self, yaml: &str) -> anyhow::Result<T::Type> {
        Ok(self.proto_repr::<T, _>(serde_yaml::Deserializer::from_str(yaml))?)
    }
}
