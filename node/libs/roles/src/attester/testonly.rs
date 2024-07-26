use super::{
    AggregateMultiSig, AggregateSignature, Batch, BatchHash, BatchNumber, BatchQC, Committee, Msg,
    MsgHash, MultiSig, PublicKey, SecretKey, Signature, Signed, Signers, SyncBatch,
    WeightedAttester,
};
use crate::validator::Payload;
use bit_vec::BitVec;
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};
use std::sync::Arc;
use zksync_consensus_crypto::bls12_381;
use zksync_consensus_utils::enum_util::Variant;

impl AggregateSignature {
    /// Generate a new aggregate signature from a list of signatures.
    pub fn aggregate<'a>(sigs: impl IntoIterator<Item = &'a bls12_381::Signature>) -> Self {
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
            number: rng.gen(),
            hash: rng.gen(),
            genesis: rng.gen(),
        }
    }
}

impl Distribution<SyncBatch> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> SyncBatch {
        let size: usize = rng.gen_range(500..1000);
        SyncBatch {
            number: rng.gen(),
            payloads: vec![Payload((0..size).map(|_| rng.gen()).collect())],
            proof: rng.gen::<[u8; 32]>().to_vec(),
        }
    }
}

impl Distribution<BatchNumber> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> BatchNumber {
        BatchNumber(rng.gen())
    }
}

impl Distribution<BatchHash> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> BatchHash {
        BatchHash(rng.gen())
    }
}

impl Distribution<BatchQC> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> BatchQC {
        BatchQC {
            message: rng.gen(),
            signatures: rng.gen(),
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

impl Distribution<MultiSig> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> MultiSig {
        let mut sig = MultiSig::default();
        for _ in 0..rng.gen_range(0..5) {
            sig.add(rng.gen(), rng.gen());
        }
        sig
    }
}

impl Distribution<AggregateSignature> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> AggregateSignature {
        AggregateSignature(rng.gen())
    }
}

impl Distribution<AggregateMultiSig> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> AggregateMultiSig {
        AggregateMultiSig {
            signers: rng.gen(),
            sig: rng.gen(),
        }
    }
}

impl<V> Distribution<Signed<V>> for Standard
where
    V: Variant<Msg>,
    Standard: Distribution<V>,
{
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Signed<V> {
        rng.gen::<SecretKey>().sign_msg(rng.gen())
    }
}

impl Distribution<MsgHash> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> MsgHash {
        MsgHash(rng.gen())
    }
}
