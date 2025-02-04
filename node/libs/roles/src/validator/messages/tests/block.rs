use crate::validator::messages::testonly;
use crate::validator::messages::{BlockNumber, Payload, PayloadHash};
use zksync_consensus_crypto::{keccak256::Keccak256, Text};

#[test]
fn payload_hash_change_detector() {
    let want: PayloadHash = Text::new(
        "payload:keccak256:ba8ffff2526cae27a9e8e014749014b08b80e01905c8b769159d02d6579d9b83",
    )
    .decode()
    .unwrap();
    assert_eq!(want, testonly::payload().hash());
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
