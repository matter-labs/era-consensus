use rand::Rng;
use zksync_concurrency::ctx;
use zksync_consensus_crypto::{ByteFmt, Text, TextFmt};
use zksync_protobuf::testonly::{test_encode, test_encode_random};

use super::*;

#[test]
fn test_byte_encoding() {
    let key = SecretKey::generate();
    assert_eq!(key, ByteFmt::decode(&ByteFmt::encode(&key)).unwrap());
    assert_eq!(
        key.public(),
        ByteFmt::decode(&ByteFmt::encode(&key.public())).unwrap()
    );
}

#[test]
fn test_text_encoding() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let key = SecretKey::generate();
    let t1 = TextFmt::encode(&key);
    let t2 = TextFmt::encode(&key.public());
    assert_eq!(key, Text::new(&t1).decode::<SecretKey>().unwrap());
    assert_eq!(key.public(), Text::new(&t2).decode().unwrap());
    assert!(Text::new(&t1).decode::<PublicKey>().is_err());
    assert!(Text::new(&t2).decode::<SecretKey>().is_err());

    let tx: TxHash = rng.gen();
    let t1 = TextFmt::encode(&tx);
    assert_eq!(tx, Text::new(&t1).decode::<TxHash>().unwrap());
}

#[test]
fn test_schema_encoding() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    test_encode_random::<Signed<SessionId>>(rng);
    test_encode_random::<Signed<Transaction>>(rng);
    let key = rng.gen::<SecretKey>().public();
    test_encode(rng, &key);
    test_encode_random::<Signature>(rng);
}

#[test]
fn test_public_verify() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let msg1: MsgHash = rng.gen();
    let msg2: MsgHash = rng.gen();
    let key1 = SecretKey::generate();
    let key2 = SecretKey::generate();
    let sig1 = key1.sign(&msg1);
    // Matching key and message.
    assert!(key1.public().verify(&msg1, &sig1).is_ok());
    // Mismatching message.
    assert!(key1.public().verify(&msg2, &sig1).is_err());
    // Mismatching key.
    assert!(key2.public().verify(&msg1, &sig1).is_err());
}
