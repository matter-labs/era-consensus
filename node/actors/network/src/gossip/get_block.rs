#![allow(unused)]
use std::collections::BTreeMap;
use zksync_concurrency::{ctx,scope,sync,oneshot};
use zksync_consensus_roles::validator;
use zksync_consensus_storage::BlockStoreState;
use anyhow::Context as _;

/// A block request.
type Call = (validator::BlockNumber, oneshot::Sender<validator::FinalBlock>);

/// Inner state of the `Queue`.
type Inner = BTreeMap<validator::BlockNumber, oneshot::Sender<validator::FinalBlock>>;

/// Queue of `get_block` calls.
pub(crate) struct Queue(sync::watch::Sender<Inner>);

impl Default for Queue {
    fn default() -> Self {
        Self(sync::watch::channel(Inner::default()).0)
    }
}

impl Queue {
    /// Adds a request to the queue and waits for it to be completed.
    /// Note: in the current implementation concurrent calls for the same block number are
    /// unsupported - second call will override the first call.
    pub(crate) async fn call(&self, ctx: &ctx::Ctx, n: validator::BlockNumber) -> ctx::Result<validator::FinalBlock> {
        let (send,recv) = oneshot::channel();
        self.0.send_if_modified(|x|{
            x.insert(n,send);
            // Send iff the lowest requested block changed.
            x.first_key_value().unwrap().0 == &n
        });
        Ok(recv.recv_or_disconnected(ctx).await?.context("call dropped")?)
    }

    /// Accepts a block request, which is contained in the available blocks.
    pub(crate) async fn accept(&self, ctx: &ctx::Ctx, available: &mut sync::watch::Receiver<BlockStoreState>) -> ctx::OrCanceled<Call> {
        let sub = &mut self.0.subscribe();
        while ctx.is_active() {
            // Wait for the lowest requested block to be available.
            // This scope is always cancelled, so we ignore the result.
            let mut block_number = None;
            let _ : Result<(),_> = scope::run!(ctx, |ctx,s| async {
                if let Some(n) = sub.borrow_and_update().first_key_value().map(|x|*x.0) {
                    let n = ctx::NoCopy(n);
                    s.spawn::<()>(async {
                        let n = n;
                        sync::wait_for(ctx, available, |a|a.contains(n.0)).await?;
                        block_number = Some(n.0);
                        Err(ctx::Canceled)
                    });
                }
                // If the lowest requested block changes, we need to restart the wait.
                sync::changed(ctx,sub).await?;
                Err(ctx::Canceled)
            }).await;
            let Some(block_number) = block_number else { continue };

            // Remove the request from the queue.
            let mut res = None;
            self.0.send_if_modified(|x|{
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
