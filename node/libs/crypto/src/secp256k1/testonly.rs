use crate::secp256k1::{AggregateSignature, PublicKey, SecretKey, Signature};
use elliptic_curve::ScalarPrimitive;
use k256::ecdsa::RecoveryId;
use rand::{
    distributions::{Distribution, Standard},
    CryptoRng, Rng, RngCore,
};

struct RngWrapper<R>(R);

impl<R: Rng> CryptoRng for RngWrapper<R> {}
impl<R: Rng> RngCore for RngWrapper<R> {
    fn next_u32(&mut self) -> u32 {
        self.0.next_u32()
    }

    fn next_u64(&mut self) -> u64 {
        self.0.next_u64()
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        self.0.fill_bytes(dest)
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand::Error> {
        self.0.try_fill_bytes(dest)
    }
}

impl Distribution<SecretKey> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> SecretKey {
        SecretKey(k256::SecretKey::random(&mut RngWrapper(rng)).into())
    }
}

impl Distribution<PublicKey> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> PublicKey {
        rng.gen::<SecretKey>().public()
    }
}

impl Distribution<Signature> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Signature {
        let rng = &mut RngWrapper(rng);
        let r: ScalarPrimitive<k256::Secp256k1> = ScalarPrimitive::random(rng);
        let s: ScalarPrimitive<k256::Secp256k1> = ScalarPrimitive::random(rng);
        let sig = k256::ecdsa::Signature::from_scalars(r.to_bytes(), s.to_bytes()).unwrap();
        let recid = rng.gen_range(0u8..=3u8);
        let recid = RecoveryId::from_byte(recid).unwrap();
        Signature { sig, recid }
    }
}

impl Distribution<AggregateSignature> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> AggregateSignature {
        let mut agg = AggregateSignature::default();
        for _ in 0..rng.gen_range(1..=5) {
            agg.add(rng.gen());
        }
        agg
    }
}
