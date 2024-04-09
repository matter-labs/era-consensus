//! Hash operations.

use ff_ce::{Field, PrimeField, SqrtField};
use num_bigint::BigUint;
use num_traits::Num;
use pairing::{
    bn256::{fq, Fq, FqRepr, G1Affine},
    CurveAffine,
};
use sha3::Digest as _;

/// Hashes an arbitrary message and maps it to an elliptic curve point in G1.
pub(crate) fn hash_to_point(msg: &[u8]) -> (G1Affine, u8) {
    let hash: [u8; 32] = sha3::Keccak256::new().chain_update(msg).finalize().into();

    let hash_num = BigUint::from_bytes_be(&hash);
    let prime_field_modulus = BigUint::from_str_radix(
        "30644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd47",
        16,
    )
    .unwrap();
    let x_num = hash_num % prime_field_modulus;
    let x_arr: [u64; 4] = x_num.to_u64_digits().try_into().unwrap();
    let mut x = Fq::from_repr(FqRepr(x_arr)).unwrap();

    for i in 0..255 {
        let p = get_point_from_x(x);
        if let Some(p) = p {
            return (p, i);
        }
        x.add_assign(&Fq::one());
    }

    // It should be statistically infeasible to finish the loop without finding a point.
    unreachable!()
}

fn get_point_from_x(x: Fq) -> Option<G1Affine> {
    // Compute x^3 + b.
    let mut x3b = x;
    x3b.square();
    x3b.mul_assign(&x);
    x3b.add_assign(&fq::B_COEFF);
    // Try find the square root.
    x3b.sqrt().map(|y| G1Affine::from_xy_unchecked(x, y))
}
