//! Test-only TCP utilities.
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};

use super::{accept, connect, ListenerAddr, Stream, RESERVED_LISTENER_ADDRS};
use crate::{ctx, scope};

/// Reserves a random port on localhost for a TCP listener.
pub fn reserve_listener() -> ListenerAddr {
    // Try to bind to an Ipv6 address, then fall back to Ipv4 if Ipv6 is not available.
    let localhost_addr = SocketAddr::from((Ipv6Addr::LOCALHOST, 0));
    let mut guard = tokio::net::TcpSocket::new_v6().unwrap();
    guard.set_reuseaddr(true).unwrap();
    guard.set_reuseport(true).unwrap();

    if guard.bind(localhost_addr).is_err() {
        let localhost_addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
        guard = tokio::net::TcpSocket::new_v4().unwrap();
        guard.set_reuseaddr(true).unwrap();
        guard.set_reuseport(true).unwrap();
        guard.bind(localhost_addr).unwrap();
    }
    let addr = guard.local_addr().unwrap();
    RESERVED_LISTENER_ADDRS.lock().unwrap().insert(addr, guard);
    ListenerAddr(addr)
}

/// Establishes a loopback TCP connection.
pub async fn pipe(ctx: &ctx::Ctx) -> (Stream, Stream) {
    let addr = reserve_listener();
    scope::run!(ctx, |ctx, s| async {
        let mut listener = addr.bind(false)?;
        let s1 = s.spawn(async { connect(ctx, *addr).await.unwrap() });
        let s2 = accept(ctx, &mut listener).await.unwrap()?;
        Ok((s1.join(ctx).await.unwrap(), s2))
    })
    .await
    .unwrap()
}
