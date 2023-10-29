#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Signature verification failure")]
    SignatureVerificationFailure,
    #[error("Aggregate signature verification failure")]
    AggregateSignatureVerificationFailure,
}
