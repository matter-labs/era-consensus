use crate::{store, AppConfig};
use rand::{
    distributions::{Distribution},
    Rng,
};
use tempfile::TempDir;
use zksync_concurrency::ctx;
use zksync_consensus_roles::{validator::testonly::Setup};
use zksync_consensus_storage::{testonly, PersistentBlockStore};
use zksync_protobuf::testonly::{FmtConv,test_encode_all_formats};
use zksync_consensus_utils::EncodeDist;

impl Distribution<AppConfig> for EncodeDist {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> AppConfig {
        AppConfig {
            server_addr: self.sample(rng),
            public_addr: self.sample(rng),
            metrics_server_addr: Some(self.sample(rng)),

            genesis: rng.gen(),

            gossip_dynamic_inbound_limit: rng.gen(),
            gossip_static_inbound: self.sample_range(rng).map(|_|rng.gen()).collect(),
            gossip_static_outbound: self.sample_range(rng).map(|_|(rng.gen(),self.sample(rng))).collect(),
            max_payload_size: rng.gen(),
        }
    }
}

#[test]
fn test_schema_encoding() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    test_encode_all_formats::<FmtConv<AppConfig>>(rng);
}

#[tokio::test]
async fn test_reopen_rocksdb() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let dir = TempDir::new().unwrap();
    let mut setup = Setup::new(rng, 3);
    setup.push_blocks(rng, 5);
    let mut want = vec![];
    for b in &setup.blocks {
        let store = store::RocksDB::open(setup.genesis.clone(), dir.path())
            .await
            .unwrap();
        store.store_next_block(ctx, b).await.unwrap();
        want.push(b.clone());
        assert_eq!(want, testonly::dump(ctx, &store).await);
    }
}
