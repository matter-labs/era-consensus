#![allow(unused)]
use std::collections::BTreeMap;

use anyhow::Context as _;
use zksync_concurrency::{ctx, oneshot, scope, sync};
use zksync_consensus_engine::BlockStoreState;
use zksync_consensus_roles::validator;

/// A block fetching request.
type BlockCall = (validator::BlockNumber, oneshot::Sender<()>);

/// Inner state of the `Queue`.
type BlockInner = BTreeMap<validator::BlockNumber, oneshot::Sender<()>>;

/// A request for a given resource.
pub(crate) enum RequestItem {
    /// Request for a block by number.
    Block(validator::BlockNumber),
}

/// Queue of block fetch request.
pub(crate) struct Queue {
    blocks: sync::watch::Sender<BlockInner>,
}

impl Default for Queue {
    fn default() -> Self {
        Self {
            blocks: sync::watch::channel(BlockInner::default()).0,
        }
    }
}

impl Queue {
    /// Returns the sorted list of currently requested blocks.
    pub(crate) fn current_blocks(&self) -> Vec<u64> {
        let mut blocks = self
            .blocks
            .subscribe()
            .borrow()
            .keys()
            .map(|x| x.0)
            .collect::<Vec<_>>();
        blocks.sort();
        blocks
    }

    /// Requests a resource from peers and waits until it is stored.
    /// Note: in the current implementation concurrent calls for the same resource number are
    /// unsupported - second call will override the first call.
    pub(crate) async fn request(&self, ctx: &ctx::Ctx, r: RequestItem) -> ctx::OrCanceled<()> {
        loop {
            let (send, recv) = oneshot::channel();
            match r {
                RequestItem::Block(n) => self.blocks.send_if_modified(|x| {
                    x.insert(n, send);
                    // Send iff the lowest requested block changed.
                    x.first_key_value().unwrap().0 == &n
                }),
            };
            match recv.recv_or_disconnected(ctx).await {
                // Return if completed.
                Ok(Ok(())) => return Ok(()),
                // Retry if failed.
                Ok(Err(sync::Disconnected)) => continue,
                // Remove the request from the queue if canceled.
                Err(ctx::Canceled) => {
                    match r {
                        RequestItem::Block(n) => self.blocks.send_if_modified(|x| {
                            let modified = x.first_key_value().is_some_and(|(k, _)| k == &n);
                            x.remove(&n);
                            // Send iff the lowest requested block changed.
                            modified
                        }),
                    };
                    return Err(ctx::Canceled);
                }
            }
        }
    }

    /// Accepts a block fetch request, which is contained in the available blocks range.
    /// Caller is responsible for fetching the block and adding it to the block store.
    pub(crate) async fn accept_block(
        &self,
        ctx: &ctx::Ctx,
        available: &mut sync::watch::Receiver<BlockStoreState>,
    ) -> ctx::OrCanceled<BlockCall> {
        let sub = &mut self.blocks.subscribe();
        while ctx.is_active() {
            // Wait for the lowest requested block to be available on the remote peer.
            // This scope is always cancelled, so we ignore the result.
            let mut block_number = None;
            let _: Result<(), _> = scope::run!(ctx, |ctx, s| async {
                if let Some(n) = sub.borrow_and_update().first_key_value().map(|x| *x.0) {
                    let n = ctx::NoCopy(n);
                    s.spawn::<()>(async {
                        let n = n;
                        sync::wait_for(ctx, available, |a| a.contains(n.0)).await?;
                        block_number = Some(n.0);
                        Err(ctx::Canceled)
                    });
                }
                // If the lowest requested block changes, we need to restart the wait.
                sync::changed(ctx, sub).await?;
                Err(ctx::Canceled)
            })
            .await;
            let Some(block_number) = block_number else {
                continue;
            };

            // Remove the request from the queue.
            let mut res = None;
            self.blocks.send_if_modified(|x| {
                res = x.remove_entry(&block_number);
                // Send iff the lowest requested block changed.
                res.is_some() && !x.is_empty()
            });
            // It may happen that someone else accepts our request faster.
            // In this case we need to wait again.
            if let Some(res) = res {
                return Ok(res);
            }
        }
        Err(ctx::Canceled)
    }
}
