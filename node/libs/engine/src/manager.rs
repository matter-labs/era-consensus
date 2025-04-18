use std::{collections::VecDeque, sync::Arc};

use anyhow::Context as _;
use tracing::Instrument;
use zksync_concurrency::{ctx, error::Wrap as _, scope, sync};
use zksync_consensus_roles::validator;

use crate::{
    block_store::BlockStore,
    metrics::{self, BLOCK_STORE},
    BlockStoreState, EngineInterface,
};

/// A wrapper around a EngineInterface which adds caching blocks in-memory
/// and other useful utilities.
#[derive(Debug)]
pub struct EngineManager {
    interface: Box<dyn EngineInterface>,
    genesis: validator::Genesis,
    block_store: sync::watch::Sender<BlockStore>,
}

impl EngineManager {
    /// Constructs a EngineManager.
    /// EngineManager takes ownership of the passed EngineInterface,
    /// i.e. caller should modify the underlying execution layer and
    /// persistent storage ONLY through the constructed EngineManager.
    pub async fn new(
        ctx: &ctx::Ctx,
        interface: Box<dyn EngineInterface>,
    ) -> ctx::Result<(Arc<Self>, EngineManagerRunner)> {
        let genesis = interface.genesis(ctx).await.wrap("persistent.genesis()")?;

        let persisted = interface.persisted().borrow().clone();
        persisted.verify(&genesis).context("state.verify()")?;

        let this = Arc::new(Self {
            block_store: sync::watch::channel(BlockStore {
                queued: persisted.clone(),
                persisted,
                cache: VecDeque::new(),
            })
            .0,
            genesis,
            interface,
        });

        Ok((this.clone(), EngineManagerRunner(this)))
    }

    /// Genesis specification for this block store.
    pub fn genesis(&self) -> &validator::Genesis {
        &self.genesis
    }

    /// Available blocks (in-memory & persisted).
    pub fn queued(&self) -> BlockStoreState {
        self.block_store.borrow().queued.clone()
    }

    /// Fetches a block (from queue or persistent storage).
    pub async fn get_block(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::Result<Option<validator::Block>> {
        {
            let block_store = self.block_store.borrow();
            if !block_store.queued.contains(number) {
                return Ok(None);
            }
            if let Some(block) = block_store.block(number) {
                return Ok(Some(block));
            }
        }

        let t = metrics::ENGINE_INTERFACE.get_block_latency.start();
        let block = self
            .interface
            .get_block(ctx, number)
            .await
            .wrap("persistent.block()")?;
        t.observe();

        Ok(Some(block))
    }

    /// Verifies a block.
    pub async fn verify_block(&self, ctx: &ctx::Ctx, block: &validator::Block) -> ctx::Result<()> {
        use validator::Block as B;

        match &block {
            B::PreGenesis(b) => {
                if b.number >= self.genesis.first_block {
                    return Err(anyhow::format_err!(
                        "external justification is allowed only for pre-genesis blocks"
                    )
                    .into());
                }
                let t = metrics::ENGINE_INTERFACE
                    .verify_pregenesis_block_latency
                    .start();
                self.interface
                    .verify_pregenesis_block(ctx, b)
                    .await
                    .context("verify_pregenesis_block()")?;
                t.observe();
            }
            B::FinalV1(b) => b.verify(&self.genesis).context("block.verify()")?,
            B::FinalV2(b) => b.verify(&self.genesis).context("block.verify()")?,
        }

        Ok(())
    }

    /// Append block to a queue to be persisted eventually.
    /// Since persisting a block may take a significant amount of time,
    /// BlockStore contains a queue of blocks waiting to be persisted.
    /// `queue_block()` adds a block to the queue as soon as all intermediate
    /// blocks are queued_state as well. Queue is unbounded, so it is caller's
    /// responsibility to manage the queue size.
    pub async fn queue_block(&self, ctx: &ctx::Ctx, block: validator::Block) -> ctx::Result<()> {
        self.verify_block(ctx, &block)
            .await
            .with_wrap(|| format!("verify_block({})", block.number()))?;

        sync::wait_for(ctx, &mut self.block_store.subscribe(), |block_store| {
            block_store.queued.next() >= block.number()
        })
        .await?;

        self.block_store
            .send_if_modified(|block_store| block_store.try_push(block));

        Ok(())
    }

    /// Waits until the queued blocks range is different than `old`.
    pub async fn wait_for_queued_change(
        &self,
        ctx: &ctx::Ctx,
        old: &BlockStoreState,
    ) -> ctx::OrCanceled<BlockStoreState> {
        sync::wait_for_some(ctx, &mut self.block_store.subscribe(), |block_store| {
            if &block_store.queued == old {
                return None;
            }
            Some(block_store.queued.clone())
        })
        .await
    }

    /// Waits until the given block is queued (in memory, or persisted).
    /// Note that it doesn't mean that the block is actually available, as old blocks might get pruned.
    #[tracing::instrument(skip_all, fields(l2_block = %number))]
    pub async fn wait_until_queued(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::OrCanceled<BlockStoreState> {
        Ok(
            sync::wait_for(ctx, &mut self.block_store.subscribe(), |block_store| {
                number < block_store.queued.next()
            })
            .await?
            .queued
            .clone(),
        )
    }

    /// Waits until the given block is stored persistently.
    /// Note that it doesn't mean that the block is actually available, as old blocks might get pruned.
    #[tracing::instrument(skip_all, fields(l2_block = %number))]
    pub async fn wait_until_persisted(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::OrCanceled<BlockStoreState> {
        Ok(sync::wait_for(
            ctx,
            &mut self.interface.persisted(),
            |persisted: &BlockStoreState| number < persisted.next(),
        )
        .await?
        .clone())
    }

    /// Verifies a payload.
    pub async fn verify_payload(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
        payload: &validator::Payload,
    ) -> ctx::Result<()> {
        let t = metrics::ENGINE_INTERFACE.verify_payload_latency.start();
        let res = self
            .interface
            .verify_payload(ctx, number, payload)
            .await
            .context("verify_payload()")?;
        t.observe();
        Ok(res)
    }

    /// Proposes a payload.
    pub async fn propose_payload(
        &self,
        ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::Result<validator::Payload> {
        let t = metrics::ENGINE_INTERFACE.propose_payload_latency.start();
        let payload = self
            .interface
            .propose_payload(ctx, number)
            .await
            .context("propose_payload()")?;
        t.observe();
        Ok(payload)
    }

    /// Gets the replica state.
    pub async fn get_state(&self, ctx: &ctx::Ctx) -> ctx::Result<validator::ReplicaState> {
        let t = metrics::ENGINE_INTERFACE.get_state_latency.start();
        let state = self.interface.get_state(ctx).await.context("get_state()")?;
        t.observe();
        Ok(state)
    }

    /// Sets the replica state.
    pub async fn set_state(
        &self,
        ctx: &ctx::Ctx,
        state: &validator::ReplicaState,
    ) -> ctx::Result<()> {
        let t = metrics::ENGINE_INTERFACE.set_state_latency.start();
        self.interface
            .set_state(ctx, state)
            .await
            .context("set_state()")?;
        t.observe();
        Ok(())
    }

    fn scrape_metrics(&self) -> metrics::BlockStore {
        let m = metrics::BlockStore::default();
        let block_store = self.block_store.borrow();
        m.next_queued_block.set(block_store.queued.next().0);
        m.next_persisted_block.set(block_store.persisted.next().0);
        m.cache_size.set(block_store.cache.len() as u64);
        m
    }
}

/// Runner of the EngineManager background tasks.
#[must_use]
#[derive(Debug, Clone)]
pub struct EngineManagerRunner(Arc<EngineManager>);

impl EngineManagerRunner {
    /// Runs the background tasks of the EngineManager.
    pub async fn run(self, ctx: &ctx::Ctx) -> anyhow::Result<()> {
        let store_ref = Arc::downgrade(&self.0);
        let _ = BLOCK_STORE.before_scrape(move || Some(store_ref.upgrade()?.scrape_metrics()));

        let res = scope::run!(ctx, |ctx, s| async {
            // Task watching the persisted state.
            s.spawn::<()>(async {
                let mut persisted = self.0.interface.persisted();
                persisted.mark_changed();
                loop {
                    async {
                        let new = sync::changed::<BlockStoreState>(ctx, &mut persisted)
                            .instrument(tracing::info_span!("wait_for_block_store_change"))
                            .await?
                            .clone();
                        sync::try_send_modify(&self.0.block_store, |block_store| {
                            block_store.update_persisted(new)
                        })?;

                        ctx::Ok(())
                    }
                    .instrument(tracing::info_span!("watch_persistent_state_iteration"))
                    .await?;
                }
            });

            // Task queueing blocks to be persisted.
            let block_store = &mut self.0.block_store.subscribe();
            let mut queue_next = validator::BlockNumber(0);
            loop {
                async {
                    let block = sync::wait_for_some(ctx, block_store, |block_store| {
                        block_store.block(queue_next.max(block_store.persisted.next()))
                    })
                    .instrument(tracing::info_span!("wait_for_next_block"))
                    .await?;
                    queue_next = block.number().next();

                    let t = metrics::ENGINE_INTERFACE.queue_next_block_latency.start();
                    self.0.interface.queue_next_block(ctx, block).await?;
                    t.observe();

                    ctx::Ok(())
                }
                .instrument(tracing::info_span!("queue_persist_block_iteration"))
                .await?;
            }
        })
        .await;

        match res {
            Ok(()) | Err(ctx::Error::Canceled(_)) => Ok(()),
            Err(ctx::Error::Internal(err)) => Err(err),
        }
    }
}
