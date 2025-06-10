use std::sync::Arc;

use rand::{
    distributions::{Distribution, Standard},
    Rng,
};
use zksync_consensus_utils::enum_util::Variant;

use super::{
    Msg, MsgHash, PublicKey, SecretKey, SessionId, Signature, Signed, Transaction, TxHash,
};

impl Distribution<MsgHash> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> MsgHash {
        MsgHash(rng.gen())
    }
}

impl<V: Variant<Msg> + Clone> Distribution<Signed<V>> for Standard
where
    Standard: Distribution<V>,
{
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Signed<V> {
        rng.gen::<SecretKey>().sign_msg(rng.gen())
    }
}

impl Distribution<Signature> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Signature {
        Signature(rng.gen())
    }
}

impl Distribution<PublicKey> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> PublicKey {
        PublicKey(rng.gen())
    }
}

impl Distribution<SecretKey> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> SecretKey {
        SecretKey(Arc::new(rng.gen()))
    }
}

impl Distribution<SessionId> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> SessionId {
        let n = rng.gen_range(10..20);
        SessionId((0..n).map(|_| rng.gen()).collect())
    }
}

impl Distribution<Transaction> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Transaction {
        let size: usize = rng.gen_range(10..100);
        Transaction((0..size).map(|_| rng.gen()).collect())
    }
}

impl Distribution<TxHash> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> TxHash {
        TxHash(rng.gen())
    }
}
