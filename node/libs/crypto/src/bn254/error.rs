/// Error type for generating and interacting with bn254.
#[derive(Debug, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("Signature verification failure")]
    SignatureVerificationFailure,
    #[error("Aggregate signature verification failure")]
    AggregateSignatureVerificationFailure,
}
