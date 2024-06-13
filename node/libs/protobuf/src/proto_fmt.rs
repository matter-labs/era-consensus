//! Canonical encoding for protobufs with known schema.
//!
//! Protobuf encoding spec: https://protobuf.dev/programming-guides/encoding/
//! By default the encoding is not canonical (multiple encodings may represent the same message).
//! Here we extend this spec to support a canonical encoding.
//!
//! Canonical encoding spec:
//! * fields are encoded in ascending order of tags.
//! * varints are encoded using minimal amount of bytes.
//! * map fields are not supported
//! * groups are not supported
//! * only proto3 syntax is supported
//!     * hence assigning a field a default value (proto2) is not supported
//! * all fields should have explicit presence (explicit optional or repeated)
//!     * hence a default value for primitive types (proto3) is not supported.
//! * a field of primitive type (I32, I64, Varint) is encoded as at most 1 TLV, depending on
//!   the number of values:
//!   * 0 values => field is not encoded at all
//!   * 1 value => field is encoded as a single I32/I64/Varint TLV.
//!   * >1 values => field is encoded as a single TLV using packed encoding (https://protobuf.dev/programming-guides/encoding/#packed)
//!     * protobuf spec allows for packed encoding only for repeated fields. We therefore don't
//!       support specifying multiple values for a singular field (which is pathological case
//!       anyway).
//!     * protobuf spec also doesn't allow for encoding repeated enum fields (which are encoded as
//!       Varint), however it mentions that it might allow for it in the future versions.
//!       TODO(gprusak): decide what we should do with those: prost implementation decodes correctly
//!       packed enums, I don't know about other protobuf implementations (rust and non-rust). We may either:
//!       * allow packed encoding for enums (despite the incompatibility with protobuf spec)
//!       * disallow packed encoding for enums specifically (although it complicates the canonical spec)
//!       * drop packed encoding altogether (at the cost of larger messages)
//!
//! We need canonical encoding to be able to compute hashes and verify signatures on
//! messages. Note that with this spec it is not possible to encode canonically a message with unknown
//! fields. It won't be a problem though if we decide on one of the following:
//! * require all communication to just use canonical encoding: then as long as we carry around the
//!   unknown fields untouched, they will stay canonical.
//! * hash/verify only messages that don't contain unknown fields: then we can just canonicalize
//!   whatever we understand and reject messages with unknown fields.
//! * drop the idea of canonical encoding altogether and pass around the received encoded message
//!   for hashing/verifying (i.e. keep the raw bytes together with the parsed message).
use anyhow::Context as _;
use prost::Message as _;
use prost_reflect::ReflectMessage;
use std::collections::BTreeMap;

/// Kilobyte.
#[allow(non_upper_case_globals)]
pub const kB: usize = 1 << 10;
/// Megabyte.
pub const MB: usize = 1 << 20;

/// Protobuf wire type.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Wire {
    /// VARINT.
    Varint,
    /// I64
    I64,
    /// LEN
    Len,
    /// I32
    I32,
}

/// Raw write type values, as defined in
/// https://protobuf.dev/programming-guides/encoding/#structure
/// VARINT
const VARINT: u32 = 0;
/// I64
const I64: u32 = 1;
/// LEN
const LEN: u32 = 2;
/// I32
const I32: u32 = 5;

impl Wire {
    /// Extracts a wire type from a tag.
    pub(crate) const fn from_tag(tag: u32) -> Option<Self> {
        match tag & 7 {
            VARINT => Some(Self::Varint),
            I64 => Some(Self::I64),
            LEN => Some(Self::Len),
            I32 => Some(Self::I32),
            _ => None,
        }
    }

    /// Converts wire type to the raw wire type value.
    pub(crate) const fn raw(self) -> u32 {
        match self {
            Self::Varint => VARINT,
            Self::I64 => I64,
            Self::Len => LEN,
            Self::I32 => I32,
        }
    }
}

impl From<prost_reflect::Kind> for Wire {
    fn from(kind: prost_reflect::Kind) -> Self {
        use prost_reflect::Kind;
        match kind {
            Kind::Int32
            | Kind::Int64
            | Kind::Uint32
            | Kind::Uint64
            | Kind::Sint32
            | Kind::Sint64
            | Kind::Bool
            | Kind::Enum(_) => Self::Varint,
            Kind::Fixed64 | Kind::Sfixed64 | Kind::Double => Self::I64,
            Kind::Fixed32 | Kind::Sfixed32 | Kind::Float => Self::I32,
            Kind::String | Kind::Bytes | Kind::Message(_) => Self::Len,
        }
    }
}

/// Wrapper of the quick_protobuf reader.
// TODO(gprusak): reimplement the reader and writer instead to make it more efficient.
pub(super) struct Reader<'a>(quick_protobuf::BytesReader, &'a [u8]);

impl<'a> Reader<'a> {
    /// Constructs a reader of a slice.
    fn new(bytes: &'a [u8]) -> Self {
        Self(quick_protobuf::BytesReader::from_bytes(bytes), bytes)
    }

    /// Reads a value of the given wire type.
    fn read(&mut self, wire: Wire) -> anyhow::Result<Vec<u8>> {
        let mut v = vec![];
        let mut w = quick_protobuf::Writer::new(&mut v);
        match wire {
            Wire::Varint => w.write_varint(self.0.read_varint64(self.1)?).unwrap(),
            Wire::I64 => w.write_fixed64(self.0.read_fixed64(self.1)?).unwrap(),
            Wire::Len => return Ok(self.0.read_bytes(self.1)?.into()),
            Wire::I32 => w.write_fixed32(self.0.read_fixed32(self.1)?).unwrap(),
        }
        Ok(v)
    }

    /// Reads a (possibly packed) field based on the expected field wire type and the actual
    /// wire type from the tag (which has been already read).
    /// The values of the field are appended to `out`.
    fn read_field(
        &mut self,
        out: &mut Vec<Vec<u8>>,
        field_wire: Wire,
        got_wire: Wire,
    ) -> anyhow::Result<()> {
        if got_wire == field_wire {
            out.push(self.read(field_wire)?);
            return Ok(());
        }
        if got_wire != Wire::Len {
            anyhow::bail!("unexpected wire type");
        }
        let mut r = Self::new(self.0.read_bytes(self.1)?);
        while !r.0.is_eof() {
            out.push(r.read(field_wire)?);
        }
        Ok(())
    }
}

/// Parses a message to a `field num -> values` map.
pub(super) fn read_fields(
    buf: &[u8],
    desc: &prost_reflect::MessageDescriptor,
) -> anyhow::Result<BTreeMap<u32, Vec<Vec<u8>>>> {
    if desc.parent_file().syntax() != prost_reflect::Syntax::Proto3 {
        anyhow::bail!("only proto3 syntax is supported");
    }
    let mut r = Reader::new(buf);
    let mut fields = BTreeMap::new();
    // Collect the field values from the reader.
    while !r.0.is_eof() {
        let tag = r.0.next_tag(r.1)?;
        let wire = Wire::from_tag(tag).context("invalid wire type")?;
        let field = desc.get_field(tag >> 3).context("unknown field")?;
        if field.is_map() {
            anyhow::bail!("maps unsupported");
        }
        if !field.is_list() && !field.supports_presence() {
            anyhow::bail!(
                "{}::{} : fields with implicit presence are not supported",
                field.parent_message().name(),
                field.name()
            );
        }
        r.read_field(
            fields.entry(field.number()).or_default(),
            field.kind().into(),
            wire,
        )?;
    }
    Ok(fields)
}

/// Converts an encoded protobuf message to its canonical form, given the descriptor of the message
/// type. Returns an error if:
/// * an unknown field is detected
/// * the message type doesn't support canonical encoding (implicit presence, map fields)
pub fn canonical_raw(
    buf: &[u8],
    desc: &prost_reflect::MessageDescriptor,
) -> anyhow::Result<Vec<u8>> {
    let mut v = vec![];
    let mut w = quick_protobuf::Writer::new(&mut v);
    // Append fields in ascending tags order.
    for (num, mut values) in read_fields(buf, desc)? {
        let fd = desc.get_field(num).unwrap();
        if values.len() > 1 && !fd.is_list() {
            anyhow::bail!("non-repeated field with multiple values");
        }
        if let prost_reflect::Kind::Message(desc) = &fd.kind() {
            for v in &mut values {
                *v = canonical_raw(v, desc)?;
            }
        }
        let wire = Wire::from(fd.kind());
        match wire {
            Wire::Varint | Wire::I64 | Wire::I32 => {
                if values.len() > 1 {
                    w.write_tag(num << 3 | LEN).unwrap();
                    w.write_bytes(&values.into_iter().flatten().collect::<Vec<_>>())
                        .unwrap();
                } else {
                    w.write_tag(num << 3 | wire.raw()).unwrap();
                    // inefficient workaround of the fact that quick_protobuf::Writer
                    // doesn't support just appending a sequence of bytes.
                    for b in &values[0] {
                        w.write_u8(*b).unwrap();
                    }
                }
            }
            Wire::Len => {
                for v in &values {
                    w.write_tag(num << 3 | LEN).unwrap();
                    w.write_bytes(v).unwrap();
                }
            }
        }
    }
    Ok(v)
}

/// Encodes a proto message of known schema (no unknown fields) into a canonical form.
pub fn canonical<T: ProtoFmt>(x: &T) -> Vec<u8> {
    let msg = x.build();
    canonical_raw(&msg.encode_to_vec(), &msg.descriptor()).unwrap()
}

/// Encodes a proto message.
/// Currently it outputs a canonical encoding, but `decode` accepts
/// non-canonical encoding as well.
/// It would simplify invariants if we required all messages to be canonical,
/// but we will see whether it is feasible once we have an RPC framework
/// in place.
pub fn encode<T: ProtoFmt>(x: &T) -> Vec<u8> {
    canonical(x)
}

/// Decodes a proto message.
pub fn decode<T: ProtoFmt>(bytes: &[u8]) -> anyhow::Result<T> {
    T::read(&<T as ProtoFmt>::Proto::decode(bytes)?)
}

/// Trait defining a proto representation for a type.
pub trait ProtoFmt: Sized {
    /// Proto message type representing Self.
    type Proto: ReflectMessage + Default;
    /// Converts Proto to Self.
    fn read(r: &Self::Proto) -> anyhow::Result<Self>;
    /// Converts Self to Proto.
    fn build(&self) -> Self::Proto;
}

/// Parses a required proto field.
pub fn read_required<T: ProtoFmt>(field: &Option<T::Proto>) -> anyhow::Result<T> {
    ProtoFmt::read(field.as_ref().context("missing field")?)
}

/// Parses an optional proto field.
pub fn read_optional<T: ProtoFmt>(field: &Option<T::Proto>) -> anyhow::Result<Option<T>> {
    field.as_ref().map(ProtoFmt::read).transpose()
}

/// Parses a repeated proto struct into a map.
///
/// The method assumes that we have a `BTreeMap<K, V>` field that we serialized
/// as `Vec<T>` into protobuf, where `T` consists of a `K::Proto` and `V::Proto`
/// field. The `k` and `v` function are getters to project `T` into the key and
/// value protobuf components, which are individually decoded into `K` and `V`.
pub fn read_map<T, K, V>(
    items: &[T],
    k: impl Fn(&T) -> &Option<K::Proto>,
    v: impl Fn(&T) -> &Option<V::Proto>,
) -> anyhow::Result<BTreeMap<K, V>>
where
    K: ProtoFmt + Ord,
    V: ProtoFmt,
{
    let items: Vec<(K, V)> = items
        .iter()
        .map(|item| {
            let k = read_required(k(item)).context("key")?;
            let v = read_required(v(item)).context("value")?;
            Ok((k, v))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    Ok(items.into_iter().collect())
}

/// Extracts a required field.
pub fn required<T>(field: &Option<T>) -> anyhow::Result<&T> {
    field.as_ref().context("missing")
}
