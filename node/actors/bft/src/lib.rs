//! # Consensus
//! This crate implements the Fastest-HotStuff algorithm that is described in an upcoming paper
//! It is a two-phase unchained consensus with quadratic view change (in number of authenticators, in number of
//! messages it is linear) and optimistic responsiveness.
//!
//! ## Node set
//! Right now, we assume that we have a static node set. In other words, we are running in proof-of-authority. When this repo is updated
//! to proof-of-stake, we will have a dynamic node set.
//!
//! ## Resources
//! - [Fast-HotStuff paper](https://arxiv.org/pdf/2010.11454.pdf)
//! - [HotStuff paper](https://arxiv.org/pdf/1803.05069.pdf)
//! - [HotStuff-2 paper](https://eprint.iacr.org/2023/397.pdf)
//! - [Notes on modern consensus algorithms](https://timroughgarden.github.io/fob21/andy.pdf)
//! - [Blog post comparing several consensus algorithms](https://decentralizedthoughts.github.io/2023-04-01-hotstuff-2/)
//! - Blog posts explaining [safety](https://seafooler.com/2022/01/24/understanding-safety-hotstuff/) and [responsiveness](https://seafooler.com/2022/04/02/understanding-responsiveness-hotstuff/)
use crate::io::{InputMessage, OutputMessage};
use inner::ConsensusInner;
use std::sync::Arc;
use zksync_concurrency::{ctx, scope};
use zksync_consensus_roles::validator;
use zksync_consensus_storage::ReplicaStore;
use zksync_consensus_utils::pipe::ActorPipe;

mod inner;
pub mod io;
mod leader;
mod metrics;
pub mod misc;
mod replica;
pub mod testonly;
#[cfg(test)]
mod tests;

/// Payload provider for the new blocks.
#[async_trait::async_trait]
pub trait PayloadSource: Send + Sync + 'static {
    /// Propose a payload for the block `block_number`.
    async fn propose(
        &self,
        ctx: &ctx::Ctx,
        block_number: validator::BlockNumber,
    ) -> ctx::Result<validator::Payload>;
}

/// Protocol version of this BFT implementation.
pub const PROTOCOL_VERSION: validator::ProtocolVersion = validator::ProtocolVersion::EARLIEST;

/// Starts the Consensus actor. It will start running, processing incoming messages and
/// sending output messages. This is a blocking method.
pub async fn run(
    ctx: &ctx::Ctx,
    mut pipe: ActorPipe<InputMessage, OutputMessage>,
    secret_key: validator::SecretKey,
    validator_set: validator::ValidatorSet,
    storage: ReplicaStore,
    payload_source: &dyn PayloadSource,
) -> anyhow::Result<()> {
    let inner = Arc::new(ConsensusInner {
        pipe: pipe.send,
        secret_key,
        validator_set,
    });
    let res = scope::run!(ctx, |ctx, s| async {
        let mut replica = replica::StateMachine::start(ctx, inner.clone(), storage).await?;
        let mut leader = leader::StateMachine::new(ctx, inner.clone());

        s.spawn_bg(leader::StateMachine::run_proposer(
            ctx,
            &inner,
            payload_source,
            leader.prepare_qc.subscribe(),
        ));

        tracing::info!("Starting consensus actor {:?}", inner.secret_key.public());

        // This is the infinite loop where the consensus actually runs. The validator waits for either
        // a message from the network or for a timeout, and processes each accordingly.
        loop {
            let input = pipe
                .recv
                .recv(&ctx.with_deadline(replica.timeout_deadline))
                .await
                .ok();

            // We check if the context is active before processing the input. If the context is not active,
            // we stop.
            if !ctx.is_active() {
                return Ok(());
            }

            let Some(InputMessage::Network(req)) = input else {
                replica.start_new_view(ctx).await?;
                continue;
            };

            use validator::ConsensusMsg as Msg;
            let res = match &req.msg.msg {
                Msg::ReplicaPrepare(_) | Msg::ReplicaCommit(_) => {
                    leader.process_input(ctx, req.msg).await
                }
                Msg::LeaderPrepare(_) | Msg::LeaderCommit(_) => {
                    replica.process_input(ctx, req.msg).await
                }
            };
            // Notify network actor that the message has been processed.
            // Ignore sending error.
            let _ = req.ack.send(());
            res?;
        }
    })
    .await;
    match res {
        Ok(()) | Err(ctx::Error::Canceled(_)) => Ok(()),
        Err(ctx::Error::Internal(err)) => Err(err),
    }
}
