use crate::proto;

use super::{
    AggregateSignature, Committee, L1Batch, L1BatchQC, Msg, MsgHash, PublicKey, SecretKey,
    Signature, SignedBatchMsg, Signers, WeightedAttester,
};
use bit_vec::BitVec;
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};
use std::sync::Arc;
use zksync_consensus_utils::enum_util::Variant;
use zksync_protobuf::ProtoFmt;

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

impl Distribution<L1Batch> for Standard {
    fn sample<R: Rng + ?Sized>(&self, _rng: &mut R) -> L1Batch {
        L1Batch::default()
    }
}

impl Distribution<L1BatchQC> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> L1BatchQC {
        L1BatchQC {
            message: rng.gen(),
            signers: rng.gen(),
            signature: rng.gen(),
        }
    }
}

impl Distribution<Msg> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Msg {
        Msg::L1Batch(rng.gen())
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

impl<V: Variant<Msg>> Distribution<SignedBatchMsg<V>> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> SignedBatchMsg<V> {
        rng.gen::<SecretKey>().sign_batch_msg(rng.gen())
    }
}

impl Distribution<MsgHash> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> MsgHash {
        MsgHash(rng.gen())
    }
}
