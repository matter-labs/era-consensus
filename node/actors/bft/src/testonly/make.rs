//! This module contains utilities that are only meant for testing purposes.
use crate::{Config, PayloadManager};
use rand::Rng as _;
use zksync_concurrency::ctx;
use zksync_consensus_roles::validator;

/// Produces random payload.
#[derive(Debug)]
pub struct RandomPayload;

#[async_trait::async_trait]
impl PayloadManager for RandomPayload {
    async fn propose(
        &self,
        ctx: &ctx::Ctx,
        _number: validator::BlockNumber,
    ) -> ctx::Result<validator::Payload> {
        let mut payload = validator::Payload(vec![0; Config::PAYLOAD_MAX_SIZE]);
        ctx.rng().fill(&mut payload.0[..]);
        Ok(payload)
    }
    async fn verify(
        &self,
        _ctx: &ctx::Ctx,
        _number: validator::BlockNumber,
        _payload: &validator::Payload,
    ) -> ctx::Result<()> {
        Ok(())
    }
}

/// propose() blocks indefinitely.
#[derive(Debug)]
pub struct PendingPayload;

#[async_trait::async_trait]
impl PayloadManager for PendingPayload {
    async fn propose(
        &self,
        ctx: &ctx::Ctx,
        _number: validator::BlockNumber,
    ) -> ctx::Result<validator::Payload> {
        ctx.canceled().await;
        Err(ctx::Canceled.into())
    }

    async fn verify(
        &self,
        _ctx: &ctx::Ctx,
        _number: validator::BlockNumber,
        _payload: &validator::Payload,
    ) -> ctx::Result<()> {
        Ok(())
    }
}

/// verify() doesn't accept any payload.
#[derive(Debug)]
pub struct RejectPayload;

#[async_trait::async_trait]
impl PayloadManager for RejectPayload {
    async fn propose(
        &self,
        _ctx: &ctx::Ctx,
        _number: validator::BlockNumber,
    ) -> ctx::Result<validator::Payload> {
        Ok(validator::Payload(vec![]))
    }

    async fn verify(
        &self,
        _ctx: &ctx::Ctx,
        _number: validator::BlockNumber,
        _payload: &validator::Payload,
    ) -> ctx::Result<()> {
        Err(anyhow::anyhow!("invalid payload").into())
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
