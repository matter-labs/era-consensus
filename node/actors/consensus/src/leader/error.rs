use thiserror::Error;

#[derive(Error, Debug)]
#[allow(clippy::missing_docs_in_private_items)]
pub(crate) enum Error {
    #[error("Received replica commit message for a proposal that we don't have.")]
    MissingProposal,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

// TODO: This is only a temporary measure because anyhow::Error doesn't implement
// PartialEq. When we change all errors to thiserror, we can just derive PartialEq.
impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Error::MissingProposal, Error::MissingProposal) | (Error::Other(_), Error::Other(_))
        )
    }
}
