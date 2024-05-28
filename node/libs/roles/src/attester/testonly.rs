use super::{
    AggregateSignature, Batch, BatchHeader, BatchNumber, BatchQC, Committee, FinalBatch, Msg,
    MsgHash, PayloadHash, PublicKey, SecretKey, Signature, Signed, Signers, WeightedAttester,
};
use bit_vec::BitVec;
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};
use std::sync::Arc;
use zksync_consensus_utils::enum_util::Variant;

impl AggregateSignature {
    /// Generate a new aggregate signature from a list of signatures.
    pub fn aggregate<'a>(sigs: impl IntoIterator<Item = &'a Signature>) -> Self {
        let mut agg = Self::default();
        for sig in sigs {
            agg.add(sig);
        }
        agg
    }
}

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

impl Distribution<AggregateSignature> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> AggregateSignature {
        AggregateSignature(rng.gen())
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
            proposal: rng.gen(),
        }
    }
}

impl Distribution<FinalBatch> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> FinalBatch {
        FinalBatch {
            payload: rng.gen(),
            justification: rng.gen(),
        }
    }
}

impl Distribution<BatchHeader> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> BatchHeader {
        BatchHeader {
            number: rng.gen(),
            payload: rng.gen(),
        }
    }
}

impl Distribution<BatchNumber> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> BatchNumber {
        BatchNumber(rng.gen())
    }
}

impl Distribution<BatchQC> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> BatchQC {
        BatchQC {
            message: rng.gen(),
            signers: rng.gen(),
            signature: rng.gen(),
        }
    }
}

impl Distribution<PayloadHash> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> PayloadHash {
        PayloadHash(rng.gen())
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
