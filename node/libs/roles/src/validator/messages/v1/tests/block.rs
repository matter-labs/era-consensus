use assert_matches::assert_matches;
use rand::Rng;
use validator::{messages::Payload, testonly::Setup};
use zksync_concurrency::ctx;

use super::*;

#[test]
fn test_final_block_verify() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let setup = Setup::new(rng, 2);

    let payload: Payload = rng.gen();
    let view_number = rng.gen();
    let commit_qc = setup.make_commit_qc_with_payload_v1(&payload, view_number);
    let mut final_block = FinalBlock::new(payload.clone(), commit_qc.clone());

    assert!(final_block
        .verify(setup.genesis_hash(), &setup.validators_schedule())
        .is_ok());

    final_block.payload = rng.gen();
    assert_matches!(
        final_block.verify(setup.genesis_hash(), &setup.validators_schedule()),
        Err(BlockValidationError::HashMismatch { .. })
    );

    final_block.justification.message.proposal.payload = final_block.payload.hash();
    assert_matches!(
        final_block.verify(setup.genesis_hash(), &setup.validators_schedule()),
        Err(BlockValidationError::Justification(_))
    );
}
