//! Context-aware wrapper around tokio::sync::oneshot channel.
use tokio::sync::oneshot;

use crate::{ctx, sync::Disconnected};

/// Sender end of the oneshot channel.
pub type Sender<T> = oneshot::Sender<T>;

/// Receiver end of the oneshot channel.
#[derive(Debug)]
pub struct Receiver<T>(oneshot::Receiver<T>);

/// Constructs a new oneshot channel.
pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let (send, recv) = oneshot::channel();
    (send, Receiver(recv))
}

impl<T> Receiver<T> {
    /// Awaits for a message from the channel.
    /// Waits for cancelation if the channel has been disconnected.
    pub async fn recv(self, ctx: &ctx::Ctx) -> ctx::OrCanceled<T> {
        ctx.wait(async move {
            match self.0.await {
                Ok(v) => v,
                Err(_) => std::future::pending().await,
            }
        })
        .await
    }

    /// Awaits for a message from the channel.
    /// Returns an error if channel is empty and disconnected.
    pub async fn recv_or_disconnected(
        self,
        ctx: &ctx::Ctx,
    ) -> ctx::OrCanceled<Result<T, Disconnected>> {
        Ok(ctx.wait(self.0).await?.map_err(|_| Disconnected))
    }
}
