//! Random message generators for testing.
//! Implementations of Distribution are supposed to generate realistic data,
//! but in fact they are "best-effort realistic" - they might need an upgrade,
//! if tests require stricter properties of the generated data.
use super::{consensus, push_validator_addrs, Arc};
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};
use zksync_consensus_roles::validator;

impl Distribution<consensus::Req> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> consensus::Req {
        consensus::Req(rng.gen())
    }
}

impl Distribution<push_validator_addrs::Req> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> push_validator_addrs::Req {
        let n = rng.gen_range(5..10);
        sync_validator_addrs::Resp(
            (0..n)
                .map(|_| {
                    let key: validator::SecretKey = rng.gen();
                    let addr: validator::NetAddress = rng.gen();
                    Arc::new(key.sign_msg(addr))
                })
                .collect(),
        )
    }
}
