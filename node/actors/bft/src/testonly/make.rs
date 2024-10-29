//! This module contains utilities that are only meant for testing purposes.
use crate::io::InputMessage;
use crate::PayloadManager;
use rand::{distributions::Standard, prelude::Distribution, Rng};
use zksync_concurrency::ctx;
use zksync_concurrency::oneshot;
use zksync_consensus_network::io::ConsensusReq;
use zksync_consensus_roles::validator;

// Generates a random InputMessage.
impl Distribution<InputMessage> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> InputMessage {
        let (send, _) = oneshot::channel();
        InputMessage::Network(ConsensusReq {
            msg: rng.gen(),
            ack: send,
        })
    }
}

/// Produces random payload of a given size.
#[derive(Debug)]
pub struct RandomPayload(pub usize);

#[async_trait::async_trait]
impl PayloadManager for RandomPayload {
    async fn propose(
        &self,
        ctx: &ctx::Ctx,
        _number: validator::BlockNumber,
    ) -> ctx::Result<validator::Payload> {
        let mut payload = validator::Payload(vec![0; self.0]);
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
