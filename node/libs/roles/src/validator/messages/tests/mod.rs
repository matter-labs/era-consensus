use rand::Rng;
use zksync_concurrency::ctx;
use zksync_consensus_crypto::{ByteFmt, Text, TextFmt};
use zksync_protobuf::testonly::test_encode_random;

use super::*;
use crate::validator::SecretKey;

mod block;
mod schedule;

/// Hardcoded view numbers.
fn views() -> impl Iterator<Item = ViewNumber> {
    [2297, 7203, 8394, 9089, 99821].into_iter().map(ViewNumber)
}

/// Hardcoded payload.
pub(crate) fn payload() -> Payload {
    Payload(
        hex::decode("57b79660558f18d56b5196053f64007030a1cb7eeadb5c32d816b9439f77edf5f6bd9d")
            .unwrap(),
    )
}

/// Hardcoded validator secret keys.
pub(crate) fn validator_keys() -> Vec<SecretKey> {
    [
        "validator:secret:bls12_381:\
         27cb45b1670a1ae8d376a85821d51c7f91ebc6e32788027a84758441aaf0a987",
        "validator:secret:bls12_381:\
         20132edc08a529e927f155e710ae7295a2a0d249f1b1f37726894d1d0d8f0d81",
        "validator:secret:bls12_381:\
         0946901f0a6650284726763b12de5da0f06df0016c8ec2144cf6b1903f1979a6",
        "validator:secret:bls12_381:\
         3143a64c079b2f50545288d7c9b282281e05c97ac043228830a9660ddd63fea3",
        "validator:secret:bls12_381:\
         5512f40d33844c1c8107aa630af764005ab6e13f6bf8edb59b4ca3683727e619",
    ]
    .iter()
    .map(|raw| Text::new(raw).decode().unwrap())
    .collect()
}

/// Hardcoded validators.
pub(crate) fn validators() -> Vec<ValidatorInfo> {
    validator_keys()
        .iter()
        .enumerate()
        .map(|(i, key)| ValidatorInfo {
            key: key.public(),
            weight: i as u64 + 10,
            leader: true,
        })
        .collect()
}

// Hardcoded validators schedule.
pub(crate) fn validators_schedule() -> Schedule {
    Schedule::new(validators(), leader_selection()).unwrap()
}

// Hardcoded leader selection.
pub(crate) fn leader_selection() -> LeaderSelection {
    LeaderSelection {
        frequency: 1,
        mode: LeaderSelectionMode::Weighted,
    }
}

/// Hardcoded genesis.
pub(crate) fn genesis_v1() -> Genesis {
    GenesisRaw {
        chain_id: ChainId(1337),
        fork_number: ForkNumber(42),
        first_block: BlockNumber(2834),
        protocol_version: ProtocolVersion(1),
        validators_schedule: Some(validators_schedule()),
    }
    .with_hash()
}

/// Hardcoded genesis.
pub(crate) fn genesis_v2() -> Genesis {
    GenesisRaw {
        chain_id: ChainId(1337),
        fork_number: ForkNumber(42),
        first_block: BlockNumber(2834),
        protocol_version: ProtocolVersion(2),
        validators_schedule: Some(validators_schedule()),
    }
    .with_hash()
}

#[test]
fn test_byte_encoding() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let msg_hash: MsgHash = rng.gen();
    assert_eq!(
        msg_hash,
        ByteFmt::decode(&ByteFmt::encode(&msg_hash)).unwrap()
    );
}

#[test]
fn test_text_encoding() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let msg_hash: MsgHash = rng.gen();
    let t = TextFmt::encode(&msg_hash);
    assert_eq!(msg_hash, Text::new(&t).decode::<MsgHash>().unwrap());

    let genesis_hash: GenesisHash = rng.gen();
    let t = TextFmt::encode(&genesis_hash);
    assert_eq!(genesis_hash, Text::new(&t).decode::<GenesisHash>().unwrap());
}

#[test]
fn test_schema_encoding() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    // In genesis.proto
    // TODO: Uncomment this when we deprecate v1
    //test_encode_random::<Genesis>(rng);
    test_encode_random::<GenesisHash>(rng);
    test_encode_random::<Schedule>(rng);
    test_encode_random::<ValidatorInfo>(rng);
    test_encode_random::<v1::WeightedValidator>(rng);
    test_encode_random::<LeaderSelection>(rng);
    test_encode_random::<LeaderSelectionMode>(rng);
    test_encode_random::<PayloadHash>(rng);
    test_encode_random::<Proposal>(rng);

    // In consensus.proto
    test_encode_random::<PreGenesisBlock>(rng);
    test_encode_random::<Block>(rng);
    test_encode_random::<ConsensusMsg>(rng);
    test_encode_random::<Msg>(rng);
    test_encode_random::<MsgHash>(rng);
    test_encode_random::<Signed<ConsensusMsg>>(rng);
    test_encode_random::<ReplicaState>(rng);
}
