//! Testonly utilities.
use super::{canonical, canonical_raw, decode, encode, read_fields, ProtoFmt, Wire};
use prost::Message as _;
use prost_reflect::ReflectMessage as _;
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};

/// Test encoding and canonical encoding properties.
#[track_caller]
pub fn test_encode<R: Rng, T: ProtoFmt + std::fmt::Debug + Eq>(rng: &mut R, x: &T) {
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
/// because `test_encode(rng,&rng::gen())` doesn't compile.
#[track_caller]
pub fn test_encode_random<R: Rng, T: ProtoFmt + std::fmt::Debug + Eq>(rng: &mut R)
where
    Standard: Distribution<T>,
{
    let msg = rng.gen::<T>();
    test_encode(rng, &msg);
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
