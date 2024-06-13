use super::{
    Batch, BatchNumber, BatchQC, Committee, Msg, MsgHash, MultiSig, PublicKey, SecretKey,
    Signature, Signed, Signers, WeightedAttester,
};
use bit_vec::BitVec;
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};
use std::sync::Arc;
use zksync_consensus_utils::enum_util::Variant;

impl Distribution<SecretKey> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> SecretKey {
        SecretKey(Arc::new(rng.gen()))
    }
}

impl Distribution<PublicKey> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> PublicKey {
        PublicKey(rng.gen())
    }
}

impl Distribution<Committee> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Committee {
        let count = rng.gen_range(1..11);
        let public_keys = (0..count).map(|_| WeightedAttester {
            key: rng.gen(),
            weight: 1,
        });
        Committee::new(public_keys).unwrap()
    }
}

impl Distribution<Batch> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Batch {
        Batch {
            number: BatchNumber(rng.gen()),
        }
    }
}

impl Distribution<BatchQC> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> BatchQC {
        let mut signatures = MultiSig::default();
        for _ in 0..rng.gen_range(0..5) {
            signatures.add(rng.gen(), rng.gen());
        }
        BatchQC {
            message: rng.gen(),
            signatures,
        }
    }
}

impl Distribution<Msg> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Msg {
        Msg::Batch(rng.gen())
    }
}

impl Distribution<Signers> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Signers {
        Signers(BitVec::from_bytes(&rng.gen::<[u8; 4]>()))
    }
}

impl Distribution<Signature> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Signature {
        Signature(rng.gen())
    }
}

impl<V: Variant<Msg>> Distribution<Signed<V>> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Signed<V> {
        rng.gen::<SecretKey>().sign_msg(rng.gen())
    }
}

impl Distribution<MsgHash> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> MsgHash {
        MsgHash(rng.gen())
    }
}
