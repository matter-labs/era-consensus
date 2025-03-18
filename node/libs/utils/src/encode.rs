//! Utilities for testing encodings.
use std::{path::PathBuf, str::FromStr};

use rand::{
    distributions::{Alphanumeric, DistString, Distribution},
    Rng,
};
use zksync_concurrency::{limiter, net, time};

/// Distribution for testing encodings.
pub struct EncodeDist {
    /// Generate configs with only required fields.
    pub required_only: bool,
    /// Generate decimal fractions for f64
    /// to avoid rounding errors of decimal encodings.
    pub decimal_fractions: bool,
}

impl EncodeDist {
    /// Returns a small non-empty range if `required_only` is false.
    /// Returns an empty range otherwise.
    pub fn sample_range(&self, rng: &mut (impl Rng + ?Sized)) -> std::ops::Range<usize> {
        if self.required_only {
            0..0
        } else {
            0..rng.gen_range(5..10)
        }
    }

    /// Returns `Some(f())` if `required_only` is false.
    /// Returns `None otherwise.
    pub fn sample_opt<T>(&self, f: impl FnOnce() -> T) -> Option<T> {
        if self.required_only {
            None
        } else {
            Some(f())
        }
    }

    /// Samples a collection of type T.
    pub fn sample_collect<T: IntoIterator + FromIterator<T::Item>>(
        &self,
        rng: &mut (impl Rng + ?Sized),
    ) -> T
    where
        EncodeDist: Distribution<T::Item>,
    {
        self.sample_range(rng).map(|_| self.sample(rng)).collect()
    }
}

impl<T> Distribution<Option<T>> for EncodeDist
where
    EncodeDist: Distribution<T>,
{
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Option<T> {
        self.sample_opt(|| self.sample(rng))
    }
}

impl Distribution<std::net::SocketAddr> for EncodeDist {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> std::net::SocketAddr {
        std::net::SocketAddr::new(std::net::IpAddr::from(rng.gen::<[u8; 16]>()), rng.gen())
    }
}

impl Distribution<net::Host> for EncodeDist {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> net::Host {
        Distribution::<std::net::SocketAddr>::sample(self, rng).into()
    }
}

impl Distribution<String> for EncodeDist {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> String {
        let n = self.sample_range(rng).len();
        Alphanumeric.sample_string(rng, n)
    }
}

impl Distribution<bool> for EncodeDist {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> bool {
        rng.gen()
    }
}
impl Distribution<u8> for EncodeDist {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> u8 {
        rng.gen()
    }
}
impl Distribution<u16> for EncodeDist {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> u16 {
        rng.gen()
    }
}
impl Distribution<u32> for EncodeDist {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> u32 {
        rng.gen()
    }
}
impl Distribution<u64> for EncodeDist {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> u64 {
        rng.gen()
    }
}
impl Distribution<i8> for EncodeDist {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> i8 {
        rng.gen()
    }
}
impl Distribution<i16> for EncodeDist {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> i16 {
        rng.gen()
    }
}
impl Distribution<i32> for EncodeDist {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> i32 {
        rng.gen()
    }
}
impl Distribution<i64> for EncodeDist {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> i64 {
        rng.gen()
    }
}
impl Distribution<usize> for EncodeDist {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> usize {
        rng.gen()
    }
}
impl Distribution<std::num::NonZeroU32> for EncodeDist {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> std::num::NonZeroU32 {
        rng.gen()
    }
}

impl Distribution<f64> for EncodeDist {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> f64 {
        if self.decimal_fractions {
            const PRECISION: usize = 1000000;
            #[allow(clippy::float_arithmetic)]
            return rng.gen_range(0..PRECISION) as f64 / PRECISION as f64;
        }
        rng.gen()
    }
}

impl Distribution<PathBuf> for EncodeDist {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> PathBuf {
        PathBuf::from_str(&Distribution::<String>::sample(self, rng)).unwrap()
    }
}

impl Distribution<time::Duration> for EncodeDist {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> time::Duration {
        time::Duration::new(self.sample(rng), self.sample(rng))
    }
}

impl Distribution<limiter::Rate> for EncodeDist {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> limiter::Rate {
        limiter::Rate {
            burst: self.sample(rng),
            refresh: self.sample(rng),
        }
    }
}
