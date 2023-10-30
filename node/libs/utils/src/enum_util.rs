//! Module defining a trait which allows for implementing code,
//! which is generic over the variants of an enum type.

/// Error returned when `Variant::extract` fails.
#[derive(Debug, thiserror::Error)]
#[error("bad enum variant")]
pub struct BadVariantError;

/// `impl Variant<E> for A` defines an unique embedding of `A` in `E`.
/// For example for
/// ```
/// enum E {
///   A(A)
///   B(B)
///   C(C)
/// }
/// ```
/// you can implement `Variant<E>` for `A` (and analogically for `B` and `C`), like this
/// ```
/// impl Variant<E> for A {
///   fn insert(self) -> E { E::A(self) }
///   fn extract(e:E) -> Result<Self,ErrBadVariant> {
///     let E::A(this) = e else { return Err(ErrBadVariant) };
///     Ok(this)
///   }
/// }
/// ```
///
/// It works just like `#[from]` in `thiserror::Error`, but additionally
/// provides a method to extract the embedded value.
/// In particular, we require that `A::extract(a.insert()) == a`.
///
/// Note that you can also define a "variant" which is a subset of enum variants, like this:
///
/// ```
/// enum AorB {
///   A(A)
///   B(B)
/// }
///
/// impl Variant<E> for AorB {
///   fn insert(self) -> E {
///     match self {
///       Self::A(a) => E::A(a),
///       Self::B(b) => E::B(b),
///     }
///   }
///   fn extract(e:E) -> Result<Self,ErrBadVariant> {
///     Ok(match e {
///       E::A(a) => Self::A(a),
///       E::B(b) => Self::B(b),
///       _ => return Err(ErrBadVariant),
///     })
///   }
/// }
/// ```
///
/// In category theory, this trait is equivalent to a notion of
/// [subobject](https://ncatlab.org/nlab/show/subobject).
pub trait Variant<Enum: Sized>: Sized {
    /// Constructs an enum value from a value of its variant.
    fn insert(self) -> Enum;
    /// Destructs the enum value expecting a particular variant.
    fn extract(e: Enum) -> Result<Self, BadVariantError>;
}
