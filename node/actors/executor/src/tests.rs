//! High-level tests for `Executor`.
use super::*;
use attestation::{AttestationStatusClient, AttestationStatusRunner};
use rand::Rng as _;
use std::sync::{atomic::AtomicU64, Mutex};
use tracing::Instrument as _;
use zksync_concurrency::{sync, testonly::abort_on_panic};
use zksync_consensus_bft as bft;
use zksync_consensus_network::testonly::{new_configs, new_fullnode};
use zksync_consensus_roles::validator::{testonly::Setup, BlockNumber};
use zksync_consensus_storage::{
    testonly::{in_memory, TestMemoryStorage},
    BlockStore,
};

fn config(cfg: &network::Config) -> Config {
    Config {
        server_addr: *cfg.server_addr,
        public_addr: cfg.public_addr.clone(),
        max_payload_size: usize::MAX,
        max_batch_size: usize::MAX,
        node_key: cfg.gossip.key.clone(),
        gossip_dynamic_inbound_limit: cfg.gossip.dynamic_inbound_limit,
        gossip_static_inbound: cfg.gossip.static_inbound.clone(),
        gossip_static_outbound: cfg.gossip.static_outbound.clone(),
        rpc: cfg.rpc.clone(),
        debug_page: None,
        batch_poll_interval: time::Duration::seconds(1),
    }
}

/// The test executors below are not running with attesters, so we just create an [AttestationStatusWatch]
/// that will never be updated.
fn never_attest(genesis: &validator::Genesis) -> Arc<AttestationStatusWatch> {
    Arc::new(AttestationStatusWatch::new(genesis.hash()))
}

fn validator(
    cfg: &network::Config,
    block_store: Arc<BlockStore>,
    batch_store: Arc<BatchStore>,
    replica_store: impl ReplicaStore,
) -> Executor {
    let attestation_status = never_attest(block_store.genesis());
    Executor {
        config: config(cfg),
        block_store,
        batch_store,
        validator: Some(Validator {
            key: cfg.validator_key.clone().unwrap(),
            replica_store: Box::new(replica_store),
            payload_manager: Box::new(bft::testonly::RandomPayload(1000)),
        }),
        attester: None,
        attestation_status,
    }
}

fn fullnode(
    cfg: &network::Config,
    block_store: Arc<BlockStore>,
    batch_store: Arc<BatchStore>,
) -> Executor {
    let attestation_status = never_attest(block_store.genesis());
    Executor {
        config: config(cfg),
        block_store,
        batch_store,
        validator: None,
        attester: None,
        attestation_status,
    }
}

#[tokio::test]
async fn test_single_validator() {
    abort_on_panic();
    let ctx = &ctx::root();
    let rng = &mut ctx.rng();

    let setup = Setup::new(rng, 1);
    let cfgs = new_configs(rng, &setup, 0);
    scope::run!(ctx, |ctx, s| async {
        let replica_store = in_memory::ReplicaStore::default();
        let store = TestMemoryStorage::new(ctx, &setup.genesis).await;
        s.spawn_bg(store.runner.run(ctx));
        s.spawn_bg(
            validator(
                &cfgs[0],
                store.blocks.clone(),
                store.batches.clone(),
                replica_store,
            )
            .run(ctx),
        );
        store
            .blocks
            .wait_until_persisted(ctx, BlockNumber(5))
            .await?;
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_many_validators() {
    abort_on_panic();
    let ctx = &ctx::root();
    let rng = &mut ctx.rng();

    let setup = Setup::new(rng, 3);
    let cfgs = new_configs(rng, &setup, 1);
    scope::run!(ctx, |ctx, s| async {
        for cfg in cfgs {
            let replica_store = in_memory::ReplicaStore::default();
            let store = TestMemoryStorage::new(ctx, &setup.genesis).await;
            s.spawn_bg(store.runner.run(ctx));
            s.spawn_bg(
                validator(
                    &cfg,
                    store.blocks.clone(),
                    store.batches.clone(),
                    replica_store,
                )
                .run(ctx),
            );

            // Spawn a task waiting for blocks to get finalized and delivered to this validator.
            s.spawn(async {
                let store = store.blocks;
                store.wait_until_persisted(ctx, BlockNumber(5)).await?;
                Ok(())
            });
        }
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_inactive_validator() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::AffineClock::new(20.0));
    let rng = &mut ctx.rng();

    let setup = Setup::new(rng, 1);
    let cfgs = new_configs(rng, &setup, 0);
    scope::run!(ctx, |ctx, s| async {
        // Spawn validator.
        let replica_store = in_memory::ReplicaStore::default();
        let store = TestMemoryStorage::new(ctx, &setup.genesis).await;
        s.spawn_bg(store.runner.run(ctx));
        s.spawn_bg(
            validator(
                &cfgs[0],
                store.blocks.clone(),
                store.batches.clone(),
                replica_store,
            )
            .run(ctx),
        );

        // Spawn a validator node, which doesn't belong to the consensus.
        // Therefore it should behave just like a fullnode.
        let store = TestMemoryStorage::new(ctx, &setup.genesis).await;
        s.spawn_bg(store.runner.run(ctx));
        let mut cfg = new_fullnode(rng, &cfgs[0]);
        cfg.validator_key = Some(rng.gen());
        let replica_store = in_memory::ReplicaStore::default();
        s.spawn_bg(
            validator(
                &cfg,
                store.blocks.clone(),
                store.batches.clone(),
                replica_store,
            )
            .run(ctx),
        );

        // Wait for blocks in inactive validator's store.
        store
            .blocks
            .wait_until_persisted(ctx, setup.genesis.first_block + 5)
            .await?;
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_fullnode_syncing_from_validator() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::AffineClock::new(20.0));
    let rng = &mut ctx.rng();

    let setup = Setup::new(rng, 1);
    let cfgs = new_configs(rng, &setup, 0);
    scope::run!(ctx, |ctx, s| async {
        // Spawn validator.
        let replica_store = in_memory::ReplicaStore::default();
        let store = TestMemoryStorage::new(ctx, &setup.genesis).await;
        s.spawn_bg(store.runner.run(ctx));
        s.spawn_bg(
            validator(
                &cfgs[0],
                store.blocks.clone(),
                store.batches.clone(),
                replica_store,
            )
            .run(ctx),
        );

        // Spawn full node.
        let store = TestMemoryStorage::new(ctx, &setup.genesis).await;
        s.spawn_bg(store.runner.run(ctx));
        s.spawn_bg(
            fullnode(
                &new_fullnode(rng, &cfgs[0]),
                store.blocks.clone(),
                store.batches.clone(),
            )
            .run(ctx),
        );

        // Wait for blocks in full node store.
        store
            .blocks
            .wait_until_persisted(ctx, setup.genesis.first_block + 5)
            .await?;
        Ok(())
    })
    .await
    .unwrap();
}

/// Test in which validator is syncing missing blocks from a full node before producing blocks.
#[tokio::test]
async fn test_validator_syncing_from_fullnode() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::AffineClock::new(20.0));
    let rng = &mut ctx.rng();

    let setup = Setup::new(rng, 1);
    let cfgs = new_configs(rng, &setup, 0);
    scope::run!(ctx, |ctx, s| async {
        // Run validator and produce some blocks.
        let replica_store = in_memory::ReplicaStore::default();
        let store = TestMemoryStorage::new(ctx, &setup.genesis).await;
        s.spawn_bg(store.runner.run(ctx));
        scope::run!(ctx, |ctx, s| async {
            s.spawn_bg(
                validator(
                    &cfgs[0],
                    store.blocks.clone(),
                    store.batches.clone(),
                    replica_store.clone(),
                )
                .run(ctx)
                .instrument(tracing::info_span!("validator")),
            );
            store
                .blocks
                .wait_until_persisted(ctx, setup.genesis.first_block + 4)
                .await?;
            Ok(())
        })
        .await
        .unwrap();

        // Spawn full node with storage used previously by validator -this ensures that
        // all finalized blocks are available: if we ran a fullnode in parallel to the
        // validator, there would be a race condition between fullnode syncing and validator
        // terminating.
        s.spawn_bg(
            fullnode(
                &new_fullnode(rng, &cfgs[0]),
                store.blocks.clone(),
                store.batches.clone(),
            )
            .run(ctx)
            .instrument(tracing::info_span!("fullnode")),
        );

        // Restart the validator with empty store (but preserved replica state) and non-trivial
        // `store.state.first`.
        // Validator should fetch the past blocks from the full node before producing next blocks.
        let last_block = store.blocks.queued().last.as_ref().unwrap().header().number;
        let store2 = TestMemoryStorage::new_store_with_first_block(
            ctx,
            &setup.genesis,
            setup.genesis.first_block + 2,
        )
        .await;
        s.spawn_bg(store2.runner.run(ctx));
        s.spawn_bg(
            validator(
                &cfgs[0],
                store2.blocks.clone(),
                store.batches.clone(),
                replica_store,
            )
            .run(ctx)
            .instrument(tracing::info_span!("validator")),
        );

        // Wait for the fullnode to fetch the new blocks.
        store
            .blocks
            .wait_until_persisted(ctx, last_block + 3)
            .await?;
        Ok(())
    })
    .await
    .unwrap();
}

/// Test that the AttestationStatusRunner initialises and then polls the status.
#[tokio::test]
async fn test_attestation_status_runner() {
    abort_on_panic();
    let _guard = zksync_concurrency::testonly::set_timeout(time::Duration::seconds(5));
    let ctx = &ctx::test_root(&ctx::AffineClock::new(10.0));
    let rng = &mut ctx.rng();

    let genesis: attester::GenesisHash = rng.gen();

    #[derive(Clone)]
    struct MockAttestationStatus {
        genesis: Arc<Mutex<attester::GenesisHash>>,
        batch_number: Arc<AtomicU64>,
    }

    #[async_trait::async_trait]
    impl AttestationStatusClient for MockAttestationStatus {
        async fn attestation_status(
            &self,
            _ctx: &ctx::Ctx,
        ) -> ctx::Result<Option<(attester::GenesisHash, attester::BatchNumber)>> {
            let curr = self
                .batch_number
                .fetch_add(1u64, std::sync::atomic::Ordering::Relaxed);
            if curr == 0 {
                // Return None initially to see that the runner will deal with it.
                Ok(None)
            } else {
                // The first actual result will be 1 on the 2nd poll.
                let genesis = *self.genesis.lock().unwrap();
                let next_batch_to_attest = attester::BatchNumber(curr);
                Ok(Some((genesis, next_batch_to_attest)))
            }
        }
    }

    let res = scope::run!(ctx, |ctx, s| async {
        let client = MockAttestationStatus {
            genesis: Arc::new(Mutex::new(genesis)),
            batch_number: Arc::new(AtomicU64::default()),
        };
        let (status, runner) = AttestationStatusRunner::init(
            ctx,
            Box::new(client.clone()),
            time::Duration::milliseconds(100),
            genesis,
        )
        .await
        .unwrap();

        let mut recv_status = status.subscribe();
        recv_status.mark_changed();

        // Check that the value has *not* been initialised to a non-default value.
        {
            let status = sync::changed(ctx, &mut recv_status).await?;
            assert!(status.next_batch_to_attest.is_none());
        }
        // Now start polling for new values. Starting in the foreground because we want it to fail in the end.
        s.spawn(runner.run(ctx));
        // Check that polling sets the value.
        {
            let status = sync::changed(ctx, &mut recv_status).await?;
            assert!(status.next_batch_to_attest.is_some());
            assert_eq!(status.next_batch_to_attest.unwrap().0, 1);
        }
        // Change the genesis returned by the client. It should cause the scope to fail.
        {
            let mut genesis = client.genesis.lock().unwrap();
            *genesis = rng.gen();
        }
        Ok(())
    })
    .await;

    match res {
        Ok(()) => panic!("expected to fail when the genesis changed"),
        Err(e) => assert!(
            e.to_string().contains("genesis changed"),
            "only expect failures due to genesis change; got: {e}"
        ),
    }
}
