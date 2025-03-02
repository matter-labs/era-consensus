use assert_matches::assert_matches;
use rand::Rng;
use zksync_concurrency::ctx;

use super::*;
use crate::validator::{testonly::Setup, Block, Payload, ViewNumber};

#[test]
fn test_leader_proposal_verify() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    // This will create equally weighted validators
    let mut setup = Setup::new(rng, 6);
    setup.push_blocks_v2(rng, 3);

    // Valid proposal
    let payload: Payload = rng.gen();
    let commit_qc = match setup.blocks.last().unwrap() {
        Block::FinalV2(block) => block.justification.clone(),
        _ => unreachable!(),
    };
    let justification = ProposalJustification::Commit(commit_qc);
    let proposal = LeaderProposal {
        proposal_payload: Some(payload.clone()),
        justification,
    };

    assert!(proposal.verify(&setup.genesis).is_ok());

    // Invalid justification
    let mut wrong_proposal = proposal.clone();
    wrong_proposal.justification = ProposalJustification::Timeout(rng.gen());

    assert_matches!(
        wrong_proposal.verify(&setup.genesis),
        Err(LeaderProposalVerifyError::Justification(_))
    );
}

#[test]
fn test_justification_get_implied_block() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let mut setup = Setup::new(rng, 6);
    setup.push_blocks_v2(rng, 3);
    let payload: Payload = rng.gen();

    // Justification with a commit QC
    let commit_qc = match setup.blocks.last().unwrap() {
        Block::FinalV2(block) => block.justification.clone(),
        _ => unreachable!(),
    };
    let justification = ProposalJustification::Commit(commit_qc);
    let proposal = LeaderProposal {
        proposal_payload: Some(payload.clone()),
        justification,
    };

    let (implied_block_number, implied_payload) =
        proposal.justification.get_implied_block(&setup.genesis);

    assert_eq!(implied_block_number, setup.next());
    assert!(implied_payload.is_none());

    // Justification with a timeout QC
    let timeout_qc = setup.make_timeout_qc_v2(rng, ViewNumber(7), Some(&payload));
    let justification = ProposalJustification::Timeout(timeout_qc);
    let proposal = LeaderProposal {
        proposal_payload: None,
        justification,
    };

    let (implied_block_number, implied_payload) =
        proposal.justification.get_implied_block(&setup.genesis);

    assert_eq!(implied_block_number, setup.next());
    assert_eq!(implied_payload, Some(payload.hash()));
}
