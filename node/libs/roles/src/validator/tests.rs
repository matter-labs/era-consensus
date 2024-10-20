use super::*;
use rand::Rng;
use zksync_concurrency::ctx;
use zksync_consensus_crypto::{ByteFmt, Text, TextFmt};
use zksync_protobuf::testonly::test_encode_random;

#[test]
fn test_byte_encoding() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let sk: SecretKey = rng.gen();
    assert_eq!(sk, ByteFmt::decode(&ByteFmt::encode(&sk)).unwrap());

    let pk: PublicKey = rng.gen();
    assert_eq!(pk, ByteFmt::decode(&ByteFmt::encode(&pk)).unwrap());

    let sig: Signature = rng.gen();
    assert_eq!(sig, ByteFmt::decode(&ByteFmt::encode(&sig)).unwrap());

    let agg_sig: AggregateSignature = rng.gen();
    assert_eq!(
        agg_sig,
        ByteFmt::decode(&ByteFmt::encode(&agg_sig)).unwrap()
    );

    let pop: ProofOfPossession = rng.gen();
    assert_eq!(pop, ByteFmt::decode(&ByteFmt::encode(&pop)).unwrap());

    let final_block: FinalBlock = rng.gen();
    assert_eq!(
        final_block,
        ByteFmt::decode(&ByteFmt::encode(&final_block)).unwrap()
    );

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

    let sk: SecretKey = rng.gen();
    let t = TextFmt::encode(&sk);
    assert_eq!(sk, Text::new(&t).decode::<SecretKey>().unwrap());

    let pk: PublicKey = rng.gen();
    let t = TextFmt::encode(&pk);
    assert_eq!(pk, Text::new(&t).decode::<PublicKey>().unwrap());

    let sig: Signature = rng.gen();
    let t = TextFmt::encode(&sig);
    assert_eq!(sig, Text::new(&t).decode::<Signature>().unwrap());

    let pop: ProofOfPossession = rng.gen();
    let t = TextFmt::encode(&pop);
    assert_eq!(pop, Text::new(&t).decode::<ProofOfPossession>().unwrap());

    let agg_sig: AggregateSignature = rng.gen();
    let t = TextFmt::encode(&agg_sig);
    assert_eq!(
        agg_sig,
        Text::new(&t).decode::<AggregateSignature>().unwrap()
    );

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
    test_encode_random::<BlockHeader>(rng);
    test_encode_random::<FinalBlock>(rng);
    test_encode_random::<PreGenesisBlock>(rng);
    test_encode_random::<Block>(rng);
    test_encode_random::<Signed<ConsensusMsg>>(rng);
    test_encode_random::<TimeoutQC>(rng);
    test_encode_random::<CommitQC>(rng);
    test_encode_random::<Msg>(rng);
    test_encode_random::<MsgHash>(rng);
    test_encode_random::<Signers>(rng);
    test_encode_random::<PublicKey>(rng);
    test_encode_random::<Signature>(rng);
    test_encode_random::<Genesis>(rng);
    test_encode_random::<AggregateSignature>(rng);
    test_encode_random::<GenesisHash>(rng);
    test_encode_random::<LeaderSelectionMode>(rng);
}
