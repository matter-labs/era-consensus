use super::*;
use rand::Rng;
use zksync_concurrency::ctx;
use zksync_consensus_crypto::{ByteFmt, Text, TextFmt};
use zksync_protobuf::testonly::test_encode_random;

mod block;
mod genesis;

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
    test_encode_random::<Signed<v1::ConsensusMsg>>(rng);
    test_encode_random::<Msg>(rng);
    test_encode_random::<MsgHash>(rng);
    test_encode_random::<Genesis>(rng);
    test_encode_random::<GenesisHash>(rng);
}
