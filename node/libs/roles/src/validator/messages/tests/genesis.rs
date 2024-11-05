use super::*;
use rand::{prelude::StdRng, Rng, SeedableRng};
use validator::testonly::Setup;
use zksync_concurrency::ctx;
use zksync_protobuf::ProtoFmt as _;

#[test]
fn genesis_verify_leader_pubkey_not_in_committee() {
    let mut rng = StdRng::seed_from_u64(29483920);
    let mut genesis = rng.gen::<GenesisRaw>();
    genesis.leader_selection = LeaderSelectionMode::Sticky(rng.gen());
    let genesis = genesis.with_hash();
    assert!(genesis.verify().is_err())
}

#[test]
fn test_genesis_verify() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    let genesis = Setup::new(rng, 1).genesis.clone();
    assert!(genesis.verify().is_ok());
    assert!(Genesis::read(&genesis.build()).is_ok());

    let mut genesis = (*genesis).clone();
    genesis.leader_selection = LeaderSelectionMode::Sticky(rng.gen());
    let genesis = genesis.with_hash();
    assert!(genesis.verify().is_err());
    assert!(Genesis::read(&genesis.build()).is_err())
}
