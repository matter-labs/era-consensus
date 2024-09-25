//! Random message generators for testing.
//! Implementations of Distribution are supposed to generate realistic data,
//! but in fact they are "best-effort realistic" - they might need an upgrade,
//! if tests require stricter properties of the generated data.
use crate::rpc;
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};
use std::sync::Arc;
use zksync_consensus_roles::validator;
use zksync_consensus_storage::{BlockStoreState};

impl Distribution<rpc::consensus::Req> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> rpc::consensus::Req {
        rpc::consensus::Req(rng.gen())
    }
}

impl Distribution<rpc::consensus::Resp> for Standard {
    fn sample<R: Rng + ?Sized>(&self, _rng: &mut R) -> rpc::consensus::Resp {
        rpc::consensus::Resp
    }
}

impl Distribution<rpc::push_validator_addrs::Req> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> rpc::push_validator_addrs::Req {
        let n = rng.gen_range(5..10);
        rpc::push_validator_addrs::Req(
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

impl Distribution<rpc::push_block_store_state::Req> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> rpc::push_block_store_state::Req {
        rpc::push_block_store_state::Req::new(BlockStoreState {
            first: rng.gen(),
            last: rng.gen(),
        },&rng.gen())
    }
}

impl Distribution<rpc::get_block::Req> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> rpc::get_block::Req {
        rpc::get_block::Req(rng.gen())
    }
}

impl Distribution<rpc::get_block::Resp> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> rpc::get_block::Resp {
        rpc::get_block::Resp(Some(rng.gen()))
    }
}
