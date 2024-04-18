use super::{
    AggregateSignature, AttesterSet, L1Batch, Msg, MsgHash, PublicKey, SecretKey, Signature,
    SignedBatchMsg,
};
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

impl Distribution<AttesterSet> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> AttesterSet {
        let count = rng.gen_range(1..11);
        let public_keys = (0..count).map(|_| rng.gen());
        AttesterSet::new(public_keys).unwrap()
    }
}

impl Distribution<L1Batch> for Standard {
    fn sample<R: Rng + ?Sized>(&self, _rng: &mut R) -> L1Batch {
        L1Batch::default()
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
