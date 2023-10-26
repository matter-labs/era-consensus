//! High-level tests for `Executor`.

use super::*;
use consensus::testonly::make_genesis;
use network::testonly::Instance;
use roles::validator::BlockNumber;
use storage::RocksdbStorage;

#[tokio::test]
async fn executing_single_validator() {
    concurrency::testonly::abort_on_panic();

    let ctx = &ctx::root();
    let rng = &mut ctx.rng();

    let mut net_configs = Instance::new_configs(rng, 1, 0);
    assert_eq!(net_configs.len(), 1);
    let net_config = net_configs.pop().unwrap();
    let consensus_config = net_config.consensus.unwrap();
    let validator_key = consensus_config.key.clone();
    let consensus_config = ConsensusConfig::from(consensus_config);

    let (genesis_block, validators) = make_genesis(&[validator_key.clone()], vec![]);
    let node_key = net_config.gossip.key.clone();
    let node_config = ExecutorConfig {
        server_addr: *net_config.server_addr,
        gossip: net_config.gossip.into(),
        genesis_block: genesis_block.clone(),
        validators: validators.clone(),
    };

    let temp_dir = tempfile::tempdir().unwrap();
    let storage = RocksdbStorage::new(ctx, &genesis_block, temp_dir.path());
    let storage = Arc::new(storage.await.unwrap());
    let (blocks_sender, mut blocks_receiver) = channel::unbounded();

    let mut executor = Executor::new(node_config, node_key, storage.clone()).unwrap();
    executor
        .set_validator(
            consensus_config,
            validator_key,
            storage.clone(),
            blocks_sender,
        )
        .unwrap();

    scope::run!(ctx, |ctx, s| async {
        s.spawn_bg(async {
            executor.run(ctx).await.or_else(|err| {
                if err.root_cause().is::<ctx::Canceled>() {
                    Ok(()) // Test has successfully finished
                } else {
                    Err(err)
                }
            })
        });

        let mut expected_block_number = BlockNumber(1);
        while expected_block_number < BlockNumber(5) {
            let final_block = blocks_receiver.recv(ctx).await?;
            tracing::trace!(number = %final_block.block.number, "Finalized new block");
            assert_eq!(final_block.block.number, expected_block_number);
            expected_block_number = expected_block_number.next();
        }
        anyhow::Ok(())
    })
    .await
    .unwrap();
}
