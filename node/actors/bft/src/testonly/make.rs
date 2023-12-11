//! This module contains utilities that are only meant for testing purposes.
use crate::{
    ConsensusInner, PayloadSource,
};
use rand::Rng as _;
use zksync_concurrency::ctx;
use zksync_consensus_roles::validator;

/// Provides payload consisting of random bytes.
pub struct RandomPayloadSource;

#[async_trait::async_trait]
impl PayloadSource for RandomPayloadSource {
    async fn propose(
        &self,
        ctx: &ctx::Ctx,
        _block_number: validator::BlockNumber,
    ) -> ctx::Result<validator::Payload> {
        let mut payload = validator::Payload(vec![0; ConsensusInner::PAYLOAD_MAX_SIZE]);
        ctx.rng().fill(&mut payload.0[..]);
        Ok(payload)
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
        header,
        payload,
        justification: validator::CommitQC::from(&signed_messages, &validator_set).unwrap(),
    };
    (final_block, validator_set)
}
