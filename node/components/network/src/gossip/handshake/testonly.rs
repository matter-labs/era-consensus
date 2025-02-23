//! Random message generators for testing.
//! Implementations of Distribution are supposed to generate realistic data,
//! but in fact they are "best-effort realistic" - they might need an upgrade,
//! if tests require stricter properties of the generated data.
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};
use zksync_consensus_roles::node;

use super::Handshake;

/// Semver has specific restrictions on how an identifier
/// should look like, so we play it safe and generate
/// a letter-only string of fixed length.
fn gen_semver_identifier<R: Rng + ?Sized>(rng: &mut R) -> String {
    (0..10).map(|_| rng.gen_range('a'..='z')).collect()
}

fn gen_semver<R: Rng + ?Sized>(rng: &mut R) -> semver::Version {
    semver::Version {
        major: rng.gen(),
        minor: rng.gen(),
        patch: rng.gen(),
        pre: semver::Prerelease::new(&gen_semver_identifier(rng)).unwrap(),
        build: semver::BuildMetadata::new(&gen_semver_identifier(rng)).unwrap(),
    }
}

impl Distribution<Handshake> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Handshake {
        let key: node::SecretKey = rng.gen();
        let session_id: node::SessionId = rng.gen();
        Handshake {
            session_id: key.sign_msg(session_id),
            genesis: rng.gen(),
            is_static: rng.gen(),
            build_version: Some(gen_semver(rng)),
        }
    }
}
