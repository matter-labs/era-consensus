//! Context-aware network utilities.
//! Built on top of `tokio::net`.
use crate::ctx;
use std::fmt;

pub mod tcp;

#[cfg(test)]
mod tests;

/// Network host address in the format "<domain/ip>:<port>".
/// NOT VALIDATED, validation happens at `Host::resolve()` call.
// TODO: for better type safety consider verifying host to be in the valid
// format in constructor.
#[derive(Clone, PartialEq)]
pub struct Host(pub String);

impl fmt::Debug for Host {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(fmt)
    }
}

impl From<std::net::SocketAddr> for Host {
    fn from(addr: std::net::SocketAddr) -> Self {
        Self(addr.to_string())
    }
}

impl Host {
    /// If host is of the form "<domain>:<port>", performs DNS resolution.
    /// If host is of the form "<ip>:<port>", just parses the SocketAddr.
    pub async fn resolve(
        &self,
        ctx: &ctx::Ctx,
    ) -> ctx::OrCanceled<std::io::Result<Vec<std::net::SocketAddr>>> {
        let host = self.0.clone();
        // Note that we may orphan a task executing the underlying `getnameinfo` call
        // if the ctx gets cancelled. This should be fine given that it is expected to finish
        // after a timeout and it doesn't affect the state of the application.
        // We don't use `tokio::net::lookup_host`, because it is not documented to be cancel-safe.
        Ok(ctx
            .wait(tokio::task::spawn_blocking(move || {
                // This should never panic, so unwrapping the task result is ok.
                Ok(std::net::ToSocketAddrs::to_socket_addrs(&host)?.collect())
            }))
            .await?
            .unwrap())
    }
}
