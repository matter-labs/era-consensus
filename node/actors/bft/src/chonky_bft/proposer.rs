use crate::{metrics, Config, OutputSender};
use std::sync::Arc;
use zksync_concurrency::{ctx, error::Wrap as _, sync};
use zksync_consensus_network::io::ConsensusInputMessage;
use zksync_consensus_roles::validator;

/// The proposer loop is responsible for proposing new blocks to the network. It watches for new
/// justifications from the replica and if it is the leader for the view, it proposes a new block.
pub(crate) async fn run_proposer(
    ctx: &ctx::Ctx,
    cfg: Arc<Config>,
    pipe: OutputSender,
    mut justification_watch: sync::watch::Receiver<Option<validator::ProposalJustification>>,
) -> ctx::Result<()> {
    loop {
        let Some(justification) = sync::changed(ctx, &mut justification_watch).await?.clone()
        else {
            continue;
        };

        // If we are not the leader for this view, skip it.
        if cfg.genesis().view_leader(justification.view().number) != cfg.secret_key.public() {
            continue;
        }

        let proposal = create_proposal(ctx, cfg.clone(), justification).await?;

        // Broadcast our proposal to all replicas (ourselves included).
        let msg = cfg
            .secret_key
            .sign_msg(validator::ConsensusMsg::LeaderProposal(proposal));

        pipe.send(ConsensusInputMessage { message: msg }.into());
    }
}

/// Creates a proposal for the given justification.
pub(crate) async fn create_proposal(
    ctx: &ctx::Ctx,
    cfg: Arc<Config>,
    justification: validator::ProposalJustification,
) -> ctx::Result<validator::LeaderProposal> {
    // Get the block number and check if this must be a reproposal.
    let (block_number, opt_block_hash) = justification.get_implied_block(cfg.genesis());

    let proposal_payload = match opt_block_hash {
        // There was some proposal last view that a subquorum of replicas
        // voted for and could have been finalized. We need to repropose it.
        Some(_) => None,
        // The previous proposal was finalized, so we can propose a new block.
        None => {
            // Defensively assume that PayloadManager cannot propose until the previous block is stored.
            // if we don't have the previous block, this call will halt until the other replicas timeout.
            // This is fine as we can just not propose anything and let our turn end. Eventually, some other
            // replica will produce some block with this block number and this function will unblock.
            if let Some(prev) = block_number.prev() {
                cfg.block_store.wait_until_persisted(ctx, prev).await?;
            }

            let payload = cfg
                .payload_manager
                .propose(ctx, block_number)
                .await
                .wrap("payload_manager.propose()")?;

            if payload.0.len() > cfg.max_payload_size {
                return Err(anyhow::format_err!(
                    "proposed payload too large: got {}B, max {}B",
                    payload.0.len(),
                    cfg.max_payload_size
                )
                .into());
            }

            metrics::METRICS
                .leader_proposal_payload_size
                .observe(payload.0.len());

            Some(payload)
        }
    };

    Ok(validator::LeaderProposal {
        proposal_payload,
        justification,
    })
}
