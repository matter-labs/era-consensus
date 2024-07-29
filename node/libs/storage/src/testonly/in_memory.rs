//! In-memory storage implementation.
use crate::{
    batch_store::BatchStoreState, BlockStoreState, PersistentBatchStore, PersistentBlockStore,
    ReplicaState,
};
use anyhow::Context as _;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};
use zksync_concurrency::{ctx, sync};
use zksync_consensus_roles::{attester, validator};

#[derive(Debug)]
struct BlockStoreInner {
    genesis: validator::Genesis,
    persisted: sync::watch::Sender<BlockStoreState>,
    blocks: Mutex<VecDeque<validator::FinalBlock>>,
}

#[derive(Debug)]
struct BatchStoreInner {
    genesis: validator::Genesis,
    persisted: sync::watch::Sender<BatchStoreState>,
    batches: Mutex<VecDeque<attester::SyncBatch>>,
    certs: Mutex<HashMap<attester::BatchNumber, attester::BatchQC>>,
}

/// In-memory block store.
#[derive(Clone, Debug)]
pub struct BlockStore(Arc<BlockStoreInner>);

/// In-memory replica store.
#[derive(Clone, Debug, Default)]
pub struct ReplicaStore(Arc<Mutex<ReplicaState>>);

/// In-memory replica store.
#[derive(Clone, Debug)]
pub struct BatchStore(Arc<BatchStoreInner>);

impl BlockStore {
    /// New In-memory `BlockStore`.
    pub fn new(genesis: validator::Genesis, first: validator::BlockNumber) -> Self {
        assert!(genesis.first_block <= first);
        Self(Arc::new(BlockStoreInner {
            genesis,
            persisted: sync::watch::channel(BlockStoreState { first, last: None }).0,
            blocks: Mutex::default(),
        }))
    }

    /// Truncates the storage to blocks `>=first`.
    pub fn truncate(&mut self, first: validator::BlockNumber) {
        let mut blocks = self.0.blocks.lock().unwrap();
        while blocks.front().map_or(false, |b| b.number() < first) {
            blocks.pop_front();
        }
        self.0.persisted.send_if_modified(|s| {
            if s.first >= first {
                return false;
            }
            if s.next() <= first {
                s.last = None;
            }
            s.first = first;
            true
        });
    }
}

impl BatchStore {
    /// New In-memory `BatchStore`.
    pub fn new(genesis: validator::Genesis, first: attester::BatchNumber) -> Self {
        Self(Arc::new(BatchStoreInner {
            genesis,
            persisted: sync::watch::channel(BatchStoreState { first, last: None }).0,
            batches: Mutex::default(),
            certs: Mutex::default(),
        }))
    }
}

#[async_trait::async_trait]
impl PersistentBlockStore for BlockStore {
    async fn genesis(&self, _ctx: &ctx::Ctx) -> ctx::Result<validator::Genesis> {
        Ok(self.0.genesis.clone())
    }

    fn persisted(&self) -> sync::watch::Receiver<BlockStoreState> {
        self.0.persisted.subscribe()
    }

    async fn block(
        &self,
        _ctx: &ctx::Ctx,
        number: validator::BlockNumber,
    ) -> ctx::Result<validator::FinalBlock> {
        let blocks = self.0.blocks.lock().unwrap();
        let front = blocks.front().context("not found")?;
        let idx = number
            .0
            .checked_sub(front.header().number.0)
            .context("not found")?;
        Ok(blocks.get(idx as usize).context("not found")?.clone())
    }

    async fn queue_next_block(
        &self,
        _ctx: &ctx::Ctx,
        block: validator::FinalBlock,
    ) -> ctx::Result<()> {
        let mut blocks = self.0.blocks.lock().unwrap();
        let want = self.0.persisted.borrow().next();
        if block.number() < want {
            // It may happen that a block gets fetched which is not needed any more.
            return Ok(());
        }
        if block.number() > want {
            // Blocks should be stored in order though.
            return Err(anyhow::anyhow!("got block {:?}, want {want:?}", block.number()).into());
        }
        self.0
            .persisted
            .send_modify(|p| p.last = Some(block.justification.clone()));
        blocks.push_back(block);
        Ok(())
    }
}

#[async_trait::async_trait]
impl PersistentBatchStore for BatchStore {
    fn persisted(&self) -> sync::watch::Receiver<BatchStoreState> {
        self.0.persisted.subscribe()
    }

    async fn last_batch_qc(&self, _ctx: &ctx::Ctx) -> ctx::Result<Option<attester::BatchQC>> {
        let certs = self.0.certs.lock().unwrap();
        let last_batch_number = certs.keys().max().unwrap();
        Ok(certs.get(last_batch_number).cloned())
    }

    async fn earliest_batch_number_to_sign(
        &self,
        _ctx: &ctx::Ctx,
    ) -> ctx::Result<Option<attester::BatchNumber>> {
        let batches = self.0.batches.lock().unwrap();
        let certs = self.0.certs.lock().unwrap();

        Ok(batches
            .iter()
            .map(|b| b.number)
            .find(|n| !certs.contains_key(n)))
    }

    async fn get_batch_to_sign(
        &self,
        ctx: &ctx::Ctx,
        number: attester::BatchNumber,
    ) -> ctx::Result<Option<attester::Batch>> {
        // Here we just produce some deterministic mock hash. The real hash is available in the database.
        // and contains a commitment to the data submitted to L1. It is *not* over SyncBatch.
        let Some(batch) = self.get_batch(ctx, number).await? else {
            return Ok(None);
        };

        let bz = zksync_protobuf::canonical(&batch);
        let hash = zksync_consensus_crypto::keccak256::Keccak256::new(&bz);

        Ok(Some(attester::Batch {
            number,
            hash: attester::BatchHash(hash),
            genesis: self.0.genesis.hash(),
        }))
    }

    async fn get_batch_qc(
        &self,
        _ctx: &ctx::Ctx,
        number: attester::BatchNumber,
    ) -> ctx::Result<Option<attester::BatchQC>> {
        let certs = self.0.certs.lock().unwrap();
        Ok(certs.get(&number).cloned())
    }

    async fn store_qc(&self, _ctx: &ctx::Ctx, qc: attester::BatchQC) -> ctx::Result<()> {
        self.0.certs.lock().unwrap().insert(qc.message.number, qc);
        Ok(())
    }

    async fn get_batch(
        &self,
        _ctx: &ctx::Ctx,
        number: attester::BatchNumber,
    ) -> ctx::Result<Option<attester::SyncBatch>> {
        let batches = self.0.batches.lock().unwrap();
        let Some(front) = batches.front() else {
            return Ok(None);
        };
        let Some(idx) = number.0.checked_sub(front.number.0) else {
            return Ok(None);
        };
        Ok(batches.get(idx as usize).cloned())
    }

    async fn queue_next_batch(
        &self,
        _ctx: &ctx::Ctx,
        batch: attester::SyncBatch,
    ) -> ctx::Result<()> {
        let mut batches = self.0.batches.lock().unwrap();
        let want = self.0.persisted.borrow().next();
        if batch.number != want {
            return Err(anyhow::anyhow!("got batch {:?}, want {want:?}", batch.number).into());
        }
        self.0
            .persisted
            .send_modify(|p| p.last = Some(batch.number));
        batches.push_back(batch);
        Ok(())
    }
}

#[async_trait::async_trait]
impl crate::ReplicaStore for ReplicaStore {
    async fn state(&self, _ctx: &ctx::Ctx) -> ctx::Result<ReplicaState> {
        Ok(self.0.lock().unwrap().clone())
    }

    async fn set_state(&self, _ctx: &ctx::Ctx, state: &ReplicaState) -> ctx::Result<()> {
        *self.0.lock().unwrap() = state.clone();
        Ok(())
    }
}
