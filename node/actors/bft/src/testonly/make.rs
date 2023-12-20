//! This module contains utilities that are only meant for testing purposes.
use std::sync::Arc;
use zksync_concurrency::ctx;
use zksync_consensus_roles::validator;
use zksync_consensus_storage::{ValidatorStore,ValidatorStoreDefault};

/// Never provides a payload.
#[derive(Debug)]
pub struct UnavailablePayloadSource(pub Arc<dyn ValidatorStore>);

#[async_trait::async_trait]
impl ValidatorStoreDefault for UnavailablePayloadSource {
    fn inner(&self) -> &dyn ValidatorStore { &*self.0 }
    async fn propose_payload(
        &self,
        ctx: &ctx::Ctx,
        _block_number: validator::BlockNumber,
    ) -> ctx::Result<validator::Payload> {
        ctx.canceled().await;
        Err(ctx::Canceled.into())
    }
}

/// Creates a genesis block with the given payload
/// and a validator set for the chain.
pub fn make_genesis(
    keys: &[validator::SecretKey],
    payload: validator::Payload,
    block_number: validator::BlockNumber,
) -> (validator::FinalBlock, validator::ValidatorSet) {
    let header = validator::BlockHeader::genesis(payload.hash(), block_number);
    let validator_set = validator::ValidatorSet::new(keys.iter().map(|k| k.public())).unwrap();
    let signed_messages: Vec<_> = keys
        .iter()
        .map(|sk| {
            sk.sign_msg(validator::ReplicaCommit {
                protocol_version: validator::ProtocolVersion::EARLIEST,
                view: validator::ViewNumber(0),
                proposal: header,
            })
        })
        .collect();
    let final_block = validator::FinalBlock {
        payload,
        justification: validator::CommitQC::from(&signed_messages, &validator_set).unwrap(),
    };
    (final_block, validator_set)
}
