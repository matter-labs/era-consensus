//! Random message generators for testing.
//! Implementations of Distribution are supposed to generate realistic data,
//! but in fact they are "best-effort realistic" - they might need an upgrade,
//! if tests require stricter properties of the generated data.
use super::Handshake;
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};
use roles::node;

impl Distribution<Handshake> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Handshake {
        let key: node::SecretKey = rng.gen();
        let session_id: node::SessionId = rng.gen();
        Handshake {
            session_id: key.sign_msg(session_id),
            is_static: rng.gen(),
        }
    }
}
