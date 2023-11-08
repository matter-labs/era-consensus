//! Hash operations.

use pairing::{
    bn256::{G1Affine, G1Compressed},
    EncodedPoint,
};
use sha2::Digest as _;

/// Hashes an arbitrary message and maps it to an elliptic curve point in G1.
pub(crate) fn hash_to_g1(msg: &[u8]) -> G1Affine {
    for i in 0..256 {
        // Hash the message with the index as suffix.
        let bytes: [u8; 32] = sha2::Sha256::new()
            .chain_update(msg)
            .chain_update((i as u32).to_be_bytes())
            .finalize()
            .into();

        // Try to get a G1 point from the hash. The probability that this works is around 1/8.
        let p = G1Compressed::from_fixed_bytes(bytes).into_affine();
        if let Ok(p) = p {
            return p;
        }
    }
    // It should be statistically infeasible to finish the loop without finding a point.
    unreachable!()
}
