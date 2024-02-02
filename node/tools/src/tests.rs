use crate::{store, AppConfig};
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};
use tempfile::TempDir;
use zksync_concurrency::ctx;
use zksync_consensus_roles::{node, validator::testonly::GenesisSetup};
use zksync_consensus_storage::{testonly, PersistentBlockStore};
use zksync_protobuf::testonly::test_encode_random;

fn make_addr<R: Rng + ?Sized>(rng: &mut R) -> std::net::SocketAddr {
    std::net::SocketAddr::new(std::net::IpAddr::from(rng.gen::<[u8; 16]>()), rng.gen())
}

impl Distribution<AppConfig> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> AppConfig {
        let (mut config, _) = AppConfig::default_for(1);
        config
            .with_server_addr(make_addr(rng))
            .with_public_addr(make_addr(rng))
            .with_metrics_server_addr(make_addr(rng))
            .with_gossip_dynamic_inbound_limit(rng.gen())
            .with_gossip_dynamic_inbound_limit(rng.gen())
            .with_max_payload_size(rng.gen());
        (0..5).into_iter().for_each(|_| {
            let _ = config.add_gossip_static_inbound(rng.gen::<node::SecretKey>().public());
        });
        (0..6).into_iter().for_each(|_| {
            let _ = config.add_gossip_static_outbound(rng.gen::<node::SecretKey>().public(), make_addr(rng));
        });
        config
    }
}

#[test]
fn test_schema_encoding() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    test_encode_random::<AppConfig>(rng);
}

#[tokio::test]
async fn test_reopen_rocksdb() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let dir = TempDir::new().unwrap();
    let mut setup = GenesisSetup::empty(rng, 3);
    setup.push_blocks(rng, 5);
    let mut want = vec![];
    for b in &setup.blocks {
        let store = store::RocksDB::open(dir.path()).await.unwrap();
        store.store_next_block(ctx, b).await.unwrap();
        want.push(b.clone());
        assert_eq!(want, testonly::dump(ctx, &store).await);
    }
}
