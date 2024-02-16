//! Util types for generating random values in tests.

use rand::Rng;

/// Generator of random values.
pub struct GenValue<'a, R: Rng> {
    /// Underlying RNG.
    pub rng: &'a mut R,
    /// Generate values with only required fields.
    pub required_only: bool,
    /// Generate decimal fractions for f64
    /// to avoid rounding errors of decimal encodings.
    pub decimal_fractions: bool,
}

impl<'a, R: Rng> GenValue<'a, R> {
    /// Generates a random value of type `C`.
    pub fn gen<C: RandomValue>(&mut self) -> C {
        C::sample(self)
    }
}

/// Types that can be used to generate a random instance.
pub trait RandomValue {
    /// Generates a random value.
    fn sample(g: &mut GenValue<impl Rng>) -> Self;
}
