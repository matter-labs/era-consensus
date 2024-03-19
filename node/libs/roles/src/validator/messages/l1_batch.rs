use crate::validator::{self, Signature};

/// A message to send by validators to the gossip network.
/// It contains the validators signature to sign the block batches to be sent to L1.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct L1BatchSignatureMsg(pub Signature);

/// A certificate for a batch of L2 blocks to be sent to L1.
/// It contains the signatures of the validators that signed the batch.
#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct L1BatchQC {
    /// The aggregate signature of the signed L1 batches.
    pub signatures: validator::AggregateSignature,
}

impl L1BatchQC {
    pub fn add(&mut self, signature: Signature) {
        self.signatures.add(&signature);
    }
}
