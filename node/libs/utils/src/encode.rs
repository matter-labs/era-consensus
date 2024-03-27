//! Utilites for testing encodings.
use rand::{
    distributions::{Alphanumeric, DistString, Distribution},
    Rng,
};
use zksync_concurrency::net;

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

impl<T> Distribution<Option<T>> for EncodeDist
where
    EncodeDist: Distribution<T>,
{
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Option<T> {
        self.sample_range(rng).map(|_| self.sample(rng)).next()
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
