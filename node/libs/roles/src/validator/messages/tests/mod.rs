use super::*;
use crate::validator::SecretKey;
use rand::Rng;
use zksync_concurrency::ctx;
use zksync_consensus_crypto::{ByteFmt, Text, TextFmt};
use zksync_protobuf::testonly::test_encode_random;

mod block;
mod committee;
mod genesis;

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
        "validator:secret:bls12_381:27cb45b1670a1ae8d376a85821d51c7f91ebc6e32788027a84758441aaf0a987",
        "validator:secret:bls12_381:20132edc08a529e927f155e710ae7295a2a0d249f1b1f37726894d1d0d8f0d81",
        "validator:secret:bls12_381:0946901f0a6650284726763b12de5da0f06df0016c8ec2144cf6b1903f1979a6",
        "validator:secret:bls12_381:3143a64c079b2f50545288d7c9b282281e05c97ac043228830a9660ddd63fea3",
        "validator:secret:bls12_381:5512f40d33844c1c8107aa630af764005ab6e13f6bf8edb59b4ca3683727e619",
    ]
    .iter()
    .map(|raw| Text::new(raw).decode().unwrap())
    .collect()
}

/// Hardcoded validator committee.
pub(crate) fn validator_committee() -> Committee {
    Committee::new(
        validator_keys()
            .iter()
            .enumerate()
            .map(|(i, key)| WeightedValidator {
                key: key.public(),
                weight: i as u64 + 10,
            }),
    )
    .unwrap()
}

/// Hardcoded genesis with no attesters.
pub(crate) fn genesis_v1() -> Genesis {
    GenesisRaw {
        chain_id: ChainId(1337),
        fork_number: ForkNumber(42),
        first_block: BlockNumber(2834),

        protocol_version: ProtocolVersion(1),
        validators: validator_committee(),
        leader_selection: v1::LeaderSelectionMode::Weighted,
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
    test_encode_random::<PayloadHash>(rng);
    test_encode_random::<PreGenesisBlock>(rng);
    test_encode_random::<Block>(rng);
    test_encode_random::<Signed<ConsensusMsg>>(rng);
    test_encode_random::<Msg>(rng);
    test_encode_random::<MsgHash>(rng);
    test_encode_random::<Genesis>(rng);
    test_encode_random::<GenesisHash>(rng);
}
