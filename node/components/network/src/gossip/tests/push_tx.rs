use std::{
    collections::{HashMap, HashSet},
    sync::{atomic::Ordering, Arc},
};

use anyhow::Context as _;
use assert_matches::assert_matches;
use pretty_assertions::assert_eq;
use rand::Rng;
use tracing::Instrument as _;
use zksync_concurrency::{
    ctx,
    error::Wrap as _,
    net, scope, sync,
    testonly::{abort_on_panic, set_timeout},
    time,
};
use zksync_consensus_engine::testonly::TestEngine;
use zksync_consensus_roles::validator;

use super::ValidatorAddrs;
use crate::{
    gossip::{handshake, validator_addrs::ValidatorAddrsWatch},
    metrics, preface, rpc, testonly,
};

#[tokio::test(flavor = "multi_thread")]
async fn test_push_tx_propagation() {
    abort_on_panic();
    let ctx = &ctx::test_root(&ctx::AffineClock::new(40.));
    let rng = &mut ctx.rng();
    let setup = validator::testonly::Setup::new(rng, 10);
    let cfgs = testonly::new_configs(rng, &setup, 1);

    scope::run!(ctx, |ctx, s| async {
        let engine = TestEngine::new(ctx, &setup).await;
        s.spawn_bg(engine.runner.run(ctx));
        let nodes: Vec<_> = cfgs
            .iter()
            .enumerate()
            .map(|(i, cfg)| {
                let (node, runner) = testonly::Instance::new(cfg.clone(), engine.manager.clone());
                s.spawn_bg(runner.run(ctx).instrument(tracing::trace_span!("node", i)));
                node
            })
            .collect();
        let want: HashMap<_, _> = cfgs
            .iter()
            .map(|cfg| {
                (
                    cfg.validator_key.as_ref().unwrap().public(),
                    *cfg.server_addr,
                )
            })
            .collect();
        for (i, node) in nodes.iter().enumerate() {
            tracing::trace!("awaiting for node[{i}] to learn validator_addrs");
            let sub = &mut node.net.gossip.validator_addrs.subscribe();
            sync::wait_for(ctx, sub, |got| want == to_addr_map(got)).await?;
        }
        Ok(())
    })
    .await
    .unwrap();
}
