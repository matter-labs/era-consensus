//! Implementation of a context-aware bounded and unbounded mpsc async channel.
//! Build on top of `tokio::sync::mpsc::(un)bounded_channel`.
//! Note that channel disconnection is not observable by default:
//! * send() always succeeds, unless canceled
//! * recv() always succeeds, unless canceled
//!
//! This
//! * simplifies the channel interface
//! * prevents users from relying on send/recv failure
//!   for terminating their tasks - context cancellation
//!   should be used instead
//! * avoids a race condition which arises naturally when 2 tasks in a scope
//!   are communicating which each other - if one of them terminates due to context
//!   cancelation and therefore closes the channel, the other one would observe
//!   either Disconnected or Canceled error. Since we got rid of Disconnected error,
//!   only Canceled error is expected.
use crate::{ctx, sync::Disconnected};
use std::{fmt, future::Future};
use tokio::sync::mpsc;

/// Sender end of the bounded channel.
pub struct Sender<T>(mpsc::Sender<T>);

impl<T> fmt::Debug for Sender<T> {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("Sender").finish()
    }
}

/// Receiver end of the bounded channel.
pub struct Receiver<T>(mpsc::Receiver<T>);

impl<T> fmt::Debug for Receiver<T> {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("Receiver").finish()
    }
}

/// Sender end of the unbounded channel.
pub struct UnboundedSender<T>(mpsc::UnboundedSender<T>);

/// Receiver end of the unbounded channel.
pub struct UnboundedReceiver<T>(mpsc::UnboundedReceiver<T>);

// derive(Clone) won't work, because Sender should be always
// cloneable, while derive would generate Clone implementation
// iff T is cloneable.
impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Clone for UnboundedSender<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> fmt::Debug for UnboundedSender<T> {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("UnboundedSender").finish()
    }
}

impl<T> fmt::Debug for UnboundedReceiver<T> {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("UnboundedReceiver").finish()
    }
}

/// Constructs a new bounded channel of size `size`.
/// `size` is required to be >0.
// TODO(gprusak): implement support for rendezvous channels (i.e. of size 0).
pub fn bounded<T>(size: usize) -> (Sender<T>, Receiver<T>) {
    let (send, recv) = mpsc::channel(size);
    (Sender(send), Receiver(recv))
}

/// Constructs a new unbounded channel.
pub fn unbounded<T>() -> (UnboundedSender<T>, UnboundedReceiver<T>) {
    let (send, recv) = mpsc::unbounded_channel();
    (UnboundedSender(send), UnboundedReceiver(recv))
}

/// Error returned by try_send when channel is full.
#[derive(Debug, thiserror::Error)]
#[error("channel is full")]
pub struct FullError;

impl<T> Sender<T> {
    /// Sends a message to the channel.
    /// It blocks while the channel is full.
    /// Drops the message silently if the channel is closed.
    pub fn send<'a>(
        &'a self,
        ctx: &'a ctx::Ctx,
        v: T,
    ) -> ctx::CtxAware<impl 'a + Future<Output = ctx::OrCanceled<()>>> {
        ctx::CtxAware(async move {
            // Context cancellation is forwarded,
            // but Disconnected error is ignored.
            let _ = ctx.wait(self.0.send(v)).await?;
            Ok(())
        })
    }

    /// Reserves a slot in the channel for sending.
    /// Returns an error if the channel has been disconnected.
    pub fn reserve_or_disconnected<'a>(
        &'a self,
        ctx: &'a ctx::Ctx,
    ) -> ctx::CtxAware<
        impl 'a + Future<Output = ctx::OrCanceled<Result<mpsc::Permit<'a, T>, Disconnected>>>,
    > {
        ctx.wait(async { self.0.reserve().await.map_err(|_| Disconnected) })
    }

    /// Tries to send a message to the channel.
    /// Returns an error if channel is full.
    pub fn try_send(&self, v: T) -> Result<(), FullError> {
        self.0.try_send(v).map_err(|_| FullError)
    }

    /// Waits until receiver is dropped.
    pub async fn closed(&self, ctx: &ctx::Ctx) -> ctx::OrCanceled<()> {
        ctx.wait(self.0.closed()).await
    }
}

impl<T> Receiver<T> {
    /// Awaits for a message from the channel.
    /// Waits for cancellation if the channel has been disconnected.
    pub fn recv<'a>(
        &'a mut self,
        ctx: &'a ctx::Ctx,
    ) -> ctx::CtxAware<impl 'a + Future<Output = ctx::OrCanceled<T>>> {
        ctx.wait(async move {
            // If the channel has been disconnected,
            // wait indefinitely.
            match self.0.recv().await {
                Some(v) => v,
                None => std::future::pending().await,
            }
        })
    }

    /// Awaits for a message from the channel.
    /// Returns an error if channel is empty and disconnected.
    pub fn recv_or_disconnected<'a>(
        &'a mut self,
        ctx: &'a ctx::Ctx,
    ) -> ctx::CtxAware<impl 'a + Future<Output = ctx::OrCanceled<Result<T, Disconnected>>>> {
        ctx.wait(async { self.0.recv().await.ok_or(Disconnected) })
    }

    /// Pops a message from a channel iff it is non-empty.
    pub fn try_recv(&mut self) -> Option<T> {
        match self.0.try_recv() {
            Ok(v) => Some(v),
            Err(_) => None,
        }
    }
}

impl<T> UnboundedSender<T> {
    /// Sends a message to the channel.
    pub fn send(&self, v: T) {
        // Ignores Disconnected error.
        let _ = self.0.send(v);
    }
}

impl<T> UnboundedReceiver<T> {
    /// Awaits a message from the channel.
    /// Waits for cancellation if the channel has been disconnected.
    pub fn recv<'a>(
        &'a mut self,
        ctx: &'a ctx::Ctx,
    ) -> ctx::CtxAware<impl 'a + Future<Output = ctx::OrCanceled<T>>> {
        ctx.wait(async move {
            // If the channel has been disconnected, wait indefinitely.
            match self.0.recv().await {
                Some(v) => v,
                None => std::future::pending().await,
            }
        })
    }

    /// Awaits for a message from the channel.
    /// Returns an error if channel is empty and disconnected.
    pub fn recv_or_disconnected<'a>(
        &'a mut self,
        ctx: &'a ctx::Ctx,
    ) -> ctx::CtxAware<impl 'a + Future<Output = ctx::OrCanceled<Result<T, Disconnected>>>> {
        ctx.wait(async { self.0.recv().await.ok_or(Disconnected) })
    }

    /// Pops a message from a channel iff it is non-empty.
    pub fn try_recv(&mut self) -> Option<T> {
        self.0.try_recv().ok()
    }
}
