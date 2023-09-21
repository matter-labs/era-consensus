//! Test-only TCP utilities.
use super::{accept, connect, ListenerAddr, Stream, RESERVED_LISTENER_ADDRS};
use crate::{ctx, scope};

/// Reserves a random port on localhost for a TCP listener.
pub fn reserve_listener() -> ListenerAddr {
    let guard = tokio::net::TcpSocket::new_v6().unwrap();
    guard.set_reuseaddr(true).unwrap();
    guard.set_reuseport(true).unwrap();
    guard.bind("[::1]:0".parse().unwrap()).unwrap();
    let addr = guard.local_addr().unwrap();
    RESERVED_LISTENER_ADDRS.lock().unwrap().insert(addr, guard);
    ListenerAddr(addr)
}

/// Establishes a loopback TCP connection.
pub async fn pipe(ctx: &ctx::Ctx) -> (Stream, Stream) {
    let addr = reserve_listener();
    scope::run!(ctx, |ctx, s| async {
        let mut listener = addr.bind()?;
        let s1 = s.spawn(async { connect(ctx, *addr).await.unwrap() });
        let s2 = accept(ctx, &mut listener).await.unwrap()?;
        Ok((s1.join(ctx).await.unwrap(), s2))
    })
    .await
    .unwrap()
}
