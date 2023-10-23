#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Signature verification failure
    #[error("Failed to decode secret key: TODO(moshababo)")]
    DecodeFailure,
    /// Signature verification failure
    #[error("Signature verification failure: {0:?}")]
    SignatureVerification(bn254::Error),
    /// Aggregate signature verification failure
    #[error("Aggregate signature verification failure:")]
    AggregateSignatureVerification,
    /// Error aggregating signatures
    #[error("Error aggregating signatures:")]
    SignatureAggregation,
}
