use crate::validator::Signature;

/// A message to send by validators to the gossip network.
/// It contains the validators signature to sign the block batches to be sent to L1.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct L1BatchSignature(pub Signature);
