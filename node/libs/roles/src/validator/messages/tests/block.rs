use super::*;
use assert_matches::assert_matches;
use rand::Rng;
use validator::testonly::Setup;
use zksync_concurrency::ctx;
use zksync_consensus_crypto::{keccak256::Keccak256, Text};

#[test]
fn payload_hash_change_detector() {
    let want: PayloadHash = Text::new(
        "payload:keccak256:ba8ffff2526cae27a9e8e014749014b08b80e01905c8b769159d02d6579d9b83",
    )
    .decode()
    .unwrap();
    assert_eq!(want, payload().hash());
}

#[test]
fn test_payload_hash() {
    let data = vec![1, 2, 3, 4];
    let payload = Payload(data.clone());
    let hash = payload.hash();
    assert_eq!(hash.0, Keccak256::new(&data));
}

#[test]
fn test_block_number_next() {
    let block_number = BlockNumber(5);
    assert_eq!(block_number.next(), BlockNumber(6));
}

#[test]
fn test_block_number_prev() {
    let block_number = BlockNumber(5);
    assert_eq!(block_number.prev(), Some(BlockNumber(4)));

    let block_number_zero = BlockNumber(0);
    assert_eq!(block_number_zero.prev(), None);
}

#[test]
fn test_block_number_add() {
    let block_number = BlockNumber(5);
    assert_eq!(block_number + 3, BlockNumber(8));
}

#[test]
fn test_final_block_verify() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let setup = Setup::new(rng, 2);

    let payload: Payload = rng.gen();
    let view_number = rng.gen();
    let commit_qc = setup.make_commit_qc_with_payload(&payload, view_number);
    let mut final_block = FinalBlock::new(payload.clone(), commit_qc.clone());

    assert!(final_block.verify(&setup.genesis).is_ok());

    final_block.payload = rng.gen();
    assert_matches!(
        final_block.verify(&setup.genesis),
        Err(BlockValidationError::HashMismatch { .. })
    );

    final_block.justification.message.proposal.payload = final_block.payload.hash();
    assert_matches!(
        final_block.verify(&setup.genesis),
        Err(BlockValidationError::Justification(_))
    );
}
