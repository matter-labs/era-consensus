//! Hash operations.

use ark_bn254::{G1Affine, G1Projective};
use ark_ec::AffineRepr as _;
use sha2::Digest as _;

/// Hashes an arbitrary message and maps it to an elliptic curve point in G1.
pub(crate) fn hash_to_g1(msg: &[u8]) -> G1Projective {
    for i in 0..100 {
        // Hash the message with the index as suffix.
        let bytes: [u8; 32] = sha2::Sha256::new()
            .chain_update(msg)
            .chain_update((i as u32).to_be_bytes())
            .finalize()
            .into();

        // Try to get a G1 point from the hash.
        let p = G1Affine::from_random_bytes(&bytes);

        if let Some(p) = p {
            return p.into();
        }
    }
    // It should be statistically infeasible to finish the loop without finding a point.
    unreachable!()
}
