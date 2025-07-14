use rand::{distributions::Distribution, Rng};
use tempfile::TempDir;
use zksync_concurrency::{ctx, sync};
use zksync_consensus_engine::{testonly, EngineInterface};
use zksync_consensus_roles::validator::testonly::Setup;
use zksync_consensus_utils::EncodeDist;
use zksync_protobuf::testonly::{test_encode_all_formats, FmtConv};

use crate::{config, engine};

impl Distribution<config::App> for EncodeDist {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> config::App {
        config::App {
            server_addr: self.sample(rng),
            public_addr: self.sample(rng),
            rpc_addr: self.sample(rng),
            metrics_server_addr: self.sample(rng),

            genesis: rng.gen(),
            max_payload_size: rng.gen(),
            max_tx_size: rng.gen(),
            view_timeout: self.sample(rng),
            validator_key: self.sample_opt(|| rng.gen()),

            node_key: rng.gen(),
            gossip_dynamic_inbound_limit: rng.gen(),
            gossip_static_inbound: self.sample_range(rng).map(|_| rng.gen()).collect(),
            gossip_static_outbound: self
                .sample_range(rng)
                .map(|_| (rng.gen(), self.sample(rng)))
                .collect(),
            debug_page: self.sample(rng),
            fetch_schedule_interval: self.sample(rng),
        }
    }
}

impl Distribution<config::Tls> for EncodeDist {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> config::Tls {
        config::Tls {
            cert_path: self.sample(rng),
            key_path: self.sample(rng),
        }
    }
}

impl Distribution<config::DebugPage> for EncodeDist {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> config::DebugPage {
        config::DebugPage {
            addr: self.sample(rng),
        }
    }
}

#[test]
fn test_schema_encoding() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    test_encode_all_formats::<FmtConv<config::DebugPage>>(rng);
    test_encode_all_formats::<FmtConv<config::App>>(rng);
}

#[tokio::test]
async fn test_reopen_rocksdb() {
    let ctx = &ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let dir = TempDir::new().unwrap();
    let mut setup = Setup::new(rng, 3);
    setup.push_blocks_v2(rng, 5);
    let mut want = vec![];
    for b in &setup.blocks {
        if b.number() < setup.first_block() {
            continue;
        }
        let store = engine::RocksDB::open(setup.genesis.clone(), dir.path())
            .await
            .unwrap();
        store.queue_next_block(ctx, b.clone()).await.unwrap();
        sync::wait_for(ctx, &mut store.persisted(), |p| p.contains(b.number()))
            .await
            .unwrap();
        want.push(b.clone());
        assert_eq!(want, testonly::dump(ctx, &store).await);
    }
}
