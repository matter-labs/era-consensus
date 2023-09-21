use rand::{
    rngs::{OsRng, StdRng},
    Rng, SeedableRng,
};
use sha2::{digest::Update as _, Digest as _};
use std::sync::atomic::{AtomicU64, Ordering};

/// Splittable Pseudorandom Number Generators using Cryptographic Hashing
/// https://publications.lib.chalmers.se/records/fulltext/183348/local_183348.pdf
///
/// Every branch of computation is assigned a unique ID in a form of a byte sequence.
/// We use a cryptographic hash of this sequence to seed a regular PRNG.
///
/// If it turns out to be too expensive, we can use SplitMix instead:
/// https://gee.cs.oswego.edu/dl/papers/oopsla14.pdf
/// which is not crypto-secure, but for tests should be good enough.
pub(super) struct SplitProvider {
    /// Hash builder of the prefix of the ID.
    builder: sha2::Sha256,
    /// Child branches constructed so far from this branch.
    branch: AtomicU64,
}

impl SplitProvider {
    fn next_builder(&self) -> sha2::Sha256 {
        let branch = self.branch.fetch_add(1, Ordering::SeqCst);
        self.builder.clone().chain(branch.to_le_bytes())
    }
}

pub(super) enum Provider {
    OsRng,
    /// We Box SplitProvider, because it has >100 bytes.
    Split(Box<SplitProvider>),
}

impl Provider {
    /// Uses OS entropy to seed the RNG. Use in prod.
    pub(super) fn real() -> Provider {
        Self::OsRng
    }
    /// Uses a splittable RNG for better determinism. Use only in tests.
    pub(super) fn test() -> Provider {
        Self::Split(
            SplitProvider {
                builder: sha2::Sha256::new().chain("Lzr81nDW8eSOMH".as_bytes()),
                branch: 0.into(),
            }
            .into(),
        )
    }
    pub(super) fn split(&self) -> Provider {
        match self {
            Self::OsRng => Self::OsRng,
            Self::Split(split) => Self::Split(
                SplitProvider {
                    builder: split.next_builder(),
                    branch: 0.into(),
                }
                .into(),
            ),
        }
    }

    pub(super) fn rng(&self) -> StdRng {
        StdRng::from_seed(match self {
            // StdRng seeded with OS entropy is what rand::thread_rng() does,
            // except that it caches the rng in thread-local memory and reseeds it
            // periodically. Here we push the responsibility of reseeding
            // (i.e. dropping the rng and creating another one) on the user.
            Self::OsRng => OsRng.gen(),
            Self::Split(split) => split.next_builder().finalize().into(),
        })
    }
}
