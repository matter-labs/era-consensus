//! Testonly utilities.
use prost::Message as _;
use prost_reflect::ReflectMessage;
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};
use zksync_consensus_utils::EncodeDist;

use super::{
    canonical, canonical_raw, decode, encode, read_fields,
    serde::{Deserialize, Serialize},
    ProtoFmt, ProtoRepr, Wire,
};

/// Test encoding and canonical encoding properties.
#[track_caller]
pub fn test_encode<R: Rng, T: ProtoFmt + std::fmt::Debug + PartialEq>(rng: &mut R, x: &T) {
    let x_encode = encode(x);
    let x_canonical = canonical(x);
    let x_shuffled = encode_shuffled(rng, x);

    let desc = &<T as ProtoFmt>::Proto::default().descriptor();
    for e in [&x_encode, &x_canonical, &x_shuffled] {
        // e is a valid encoding.
        assert_eq!(x, &decode::<T>(e).unwrap());
        // canonical encoding is consistent.
        assert_eq!(x_canonical, canonical_raw(e, desc).unwrap());
    }
}

/// Syntax sugar for `test_encode`,
/// because `test_encode(rng,&rng.gen())` doesn't compile.
#[track_caller]
pub fn test_encode_random<T: ProtoFmt + std::fmt::Debug + PartialEq>(rng: &mut impl Rng)
where
    Standard: Distribution<T>,
{
    for _ in 0..10 {
        let msg = rng.gen::<T>();
        test_encode(rng, &msg);
    }
}

/// shuffles recursively the order of fields in a protobuf encoding.
fn encode_shuffled_raw<R: Rng>(
    rng: &mut R,
    buf: &[u8],
    desc: &prost_reflect::MessageDescriptor,
) -> anyhow::Result<Vec<u8>> {
    // Vector of field values, values of each field are in reverse order.
    // This way we can pop the next value of the field in O(1).
    let mut fields: Vec<_> = read_fields(buf, desc)?.into_iter().collect();
    for f in &mut fields {
        f.1.reverse();
    }

    // Append fields in random order.
    let mut v = vec![];
    let mut w = quick_protobuf::Writer::new(&mut v);
    while !fields.is_empty() {
        // Select a random field.
        let i = rng.gen_range(0..fields.len());
        // Select the next value of this field.
        // Note that the values are stored in reverse order.
        let Some(mut v) = fields[i].1.pop() else {
            let last = fields.len() - 1;
            fields.swap(i, last);
            fields.pop();
            continue;
        };
        let num = fields[i].0;
        let fd = desc.get_field(num).unwrap();
        if let prost_reflect::Kind::Message(desc) = &fd.kind() {
            v = encode_shuffled_raw(rng, &v, desc)?;
        }
        let wire = Wire::from(fd.kind());
        w.write_tag(num << 3 | wire.raw()).unwrap();
        match wire {
            Wire::Varint | Wire::I64 | Wire::I32 => {
                // inefficient workaround of the fact that quick_protobuf::Writer
                // doesn't support just appending a sequence of bytes.
                for b in &v {
                    w.write_u8(*b).unwrap();
                }
            }
            Wire::Len => w.write_bytes(&v).unwrap(),
        }
    }
    Ok(v)
}

/// Encodes a proto message of known schema in a random encoding order of fields.
pub(crate) fn encode_shuffled<T: ProtoFmt, R: Rng>(rng: &mut R, x: &T) -> Vec<u8> {
    let msg = x.build();
    encode_shuffled_raw(rng, &msg.encode_to_vec(), &msg.descriptor()).unwrap()
}

/// Generalization of `ProtoFmt` and `ProtoRepr`.
pub trait ProtoConv {
    /// Type.
    type Type;
    /// Proto.
    type Proto: ReflectMessage + Default;
    /// read.
    fn read(r: &Self::Proto) -> anyhow::Result<Self::Type>;
    /// build.
    fn build(this: &Self::Type) -> Self::Proto;
}

fn encode_proto<X: ProtoConv>(msg: &X::Type) -> Vec<u8> {
    let msg = X::build(msg);
    canonical_raw(&msg.encode_to_vec(), &msg.descriptor()).unwrap()
}

fn decode_proto<X: ProtoConv>(bytes: &[u8]) -> anyhow::Result<X::Type> {
    X::read(&X::Proto::decode(bytes)?)
}

fn encode_json<X: ProtoConv>(msg: &X::Type) -> String {
    let mut s = serde_json::Serializer::pretty(vec![]);
    Serialize.proto(&X::build(msg), &mut s).unwrap();
    String::from_utf8(s.into_inner()).unwrap()
}

fn decode_json<X: ProtoConv>(json: &str) -> anyhow::Result<X::Type> {
    let mut d = serde_json::Deserializer::from_str(json);
    X::read(
        &Deserialize {
            deny_unknown_fields: true,
        }
        .proto(&mut d)?,
    )
}

fn encode_yaml<X: ProtoConv>(msg: &X::Type) -> String {
    let mut s = serde_yaml::Serializer::new(vec![]);
    Serialize.proto(&X::build(msg), &mut s).unwrap();
    String::from_utf8(s.into_inner().unwrap()).unwrap()
}

fn decode_yaml<X: ProtoConv>(yaml: &str) -> anyhow::Result<X::Type> {
    let d = serde_yaml::Deserializer::from_str(yaml);
    X::read(
        &Deserialize {
            deny_unknown_fields: true,
        }
        .proto(d)?,
    )
}

/// Wrapper for `ProtoRepr`, implementing ProtoConv;
pub struct ReprConv<P: ProtoRepr>(std::marker::PhantomData<P>);
/// Wrapper for `ProtoFmt`, implementing ProtoConv;
pub struct FmtConv<T: ProtoFmt>(std::marker::PhantomData<T>);

impl<T: ProtoFmt> ProtoConv for FmtConv<T> {
    type Type = T;
    type Proto = T::Proto;
    fn read(r: &T::Proto) -> anyhow::Result<T> {
        ProtoFmt::read(r)
    }
    fn build(this: &T) -> T::Proto {
        ProtoFmt::build(this)
    }
}

impl<P: ProtoRepr> ProtoConv for ReprConv<P> {
    type Type = P::Type;
    type Proto = P;
    fn read(r: &P) -> anyhow::Result<P::Type> {
        ProtoRepr::read(r)
    }
    fn build(this: &P::Type) -> P {
        ProtoRepr::build(this)
    }
}

/// Test reencoding random values in various formats.
#[track_caller]
pub fn test_encode_all_formats<X: ProtoConv>(rng: &mut impl Rng)
where
    X::Type: std::fmt::Debug + PartialEq,
    EncodeDist: Distribution<X::Type>,
{
    for _ in 0..10 {
        for required_only in [false, true] {
            let want: X::Type = EncodeDist {
                required_only,
                decimal_fractions: false,
            }
            .sample(rng);
            let got = decode_proto::<X>(&encode_proto::<X>(&want)).unwrap();
            assert_eq!(&want, &got, "binary encoding");
            let got = decode_yaml::<X>(&encode_yaml::<X>(&want)).unwrap();
            assert_eq!(&want, &got, "yaml encoding");

            let want: X::Type = EncodeDist {
                required_only,
                decimal_fractions: true,
            }
            .sample(rng);
            let got = decode_json::<X>(&encode_json::<X>(&want)).unwrap();
            assert_eq!(&want, &got, "json encoding");
        }
    }
}
