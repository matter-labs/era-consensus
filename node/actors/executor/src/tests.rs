//! High-level tests for `Executor`.
use super::*;
use zksync_concurrency::testonly::abort_on_panic;
use zksync_consensus_bft as bft;
use zksync_consensus_network::testonly::{new_configs, new_fullnode};
use zksync_consensus_roles::validator::{testonly::Setup, BlockNumber};
use zksync_consensus_storage::{
    testonly::{in_memory, new_store},
    BlockStore,
};

fn make_executor(cfg: &network::Config, block_store: Arc<BlockStore>) -> Executor {
    Executor {
        config: Config {
            server_addr: *cfg.server_addr,
            public_addr: cfg.public_addr,
            max_payload_size: usize::MAX,
            node_key: cfg.gossip.key.clone(),
            gossip_dynamic_inbound_limit: cfg.gossip.dynamic_inbound_limit,
            gossip_static_inbound: cfg.gossip.static_inbound.clone(),
            gossip_static_outbound: cfg.gossip.static_outbound.clone(),
        },
        block_store,
        validator: cfg.validator_key.as_ref().map(|key| Validator {
            key: key.clone(),
            replica_store: Box::new(in_memory::ReplicaStore::default()),
            payload_manager: Box::new(bft::testonly::RandomPayload(1000)),
        }),
    }
}

#[tokio::test]
async fn executing_single_validator() {
    abort_on_panic();
    let ctx = &ctx::root();
    let rng = &mut ctx.rng();

    let setup = Setup::new(rng, 1);
    let cfgs = new_configs(rng, &setup, 0);
    scope::run!(ctx, |ctx, s| async {
        let (store, runner) = new_store(ctx, &setup.genesis).await;
        s.spawn_bg(runner.run(ctx));
        s.spawn_bg(make_executor(&cfgs[0], store.clone()).run(ctx));
        store.wait_until_persisted(ctx, BlockNumber(5)).await?;
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn executing_validator_and_full_node() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::AffineClock::new(20.0));
    let rng = &mut ctx.rng();

    let setup = Setup::new(rng, 1);
    let cfgs = new_configs(rng, &setup, 0);
    scope::run!(ctx, |ctx, s| async {
        // Spawn validator.
        let (store, runner) = new_store(ctx, &setup.genesis).await;
        s.spawn_bg(runner.run(ctx));
        s.spawn_bg(make_executor(&cfgs[0], store).run(ctx));

        // Spawn full node.
        let (store, runner) = new_store(ctx, &setup.genesis).await;
        s.spawn_bg(runner.run(ctx));
        s.spawn_bg(make_executor(&new_fullnode(rng, &cfgs[0]), store.clone()).run(ctx));

        // Wait for blocks in full node store.
        store.wait_until_persisted(ctx, BlockNumber(5)).await?;
        Ok(())
    })
    .await
    .unwrap();
}
