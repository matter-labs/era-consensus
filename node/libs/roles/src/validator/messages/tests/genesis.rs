use super::*;
use crate::validator::{testonly::Setup, Genesis, GenesisHash, GenesisRaw};
use rand::{prelude::StdRng, Rng, SeedableRng};
use zksync_concurrency::ctx;
use zksync_consensus_crypto::Text;
use zksync_protobuf::ProtoFmt as _;

/// Note that genesis is NOT versioned by ProtocolVersion.
/// Even if it was, ALL versions of genesis need to be supported FOREVER,
/// unless we introduce dynamic regenesis.
#[test]
fn genesis_hash_change_detector_v1() {
    let want: GenesisHash = Text::new(
        "genesis_hash:keccak256:75cfa582fcda9b5da37af8fb63a279f777bb17a97a50519e1a61aad6c77a522f",
    )
    .decode()
    .unwrap();
    assert_eq!(want, genesis_v1().hash());
}

#[test]
fn genesis_verify_leader_pubkey_not_in_committee() {
    let mut rng = StdRng::seed_from_u64(29483920);
    let mut genesis = rng.gen::<GenesisRaw>();
    genesis.leader_selection = v1::LeaderSelectionMode::Sticky(rng.gen());
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
    genesis.leader_selection = v1::LeaderSelectionMode::Sticky(rng.gen());
    let genesis = genesis.with_hash();
    assert!(genesis.verify().is_err());
    assert!(Genesis::read(&genesis.build()).is_err())
}
