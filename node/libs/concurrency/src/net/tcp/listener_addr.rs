//! TCP listener socket address.
//! Supports reserving a socket address without actually opening the socket,
//! which is useful in tests.
use std::{collections::HashMap, fmt, sync::Mutex};

use once_cell::sync::Lazy;

use super::Listener;

/// Queue size of the TCP listener socket.
const LISTENER_BACKLOG: u32 = 32;

/// Guards ensuring that OS considers the given TCP listener port to be in use until
/// this OS process is terminated.
/// Used in tests to prevent losing the port when the server is restarted.
pub(super) static RESERVED_LISTENER_ADDRS: Lazy<
    Mutex<HashMap<std::net::SocketAddr, tokio::net::TcpSocket>>,
> = Lazy::new(|| Mutex::new(HashMap::new()));

/// ListenerAddr is isomorphic to std::net::SocketAddr, but it should be used
/// solely for opening a TCP listener socket on it.
///
/// In tests it additionally allows to "reserve" a random unused TCP port:
/// * it allows to avoid race conditions in tests which require a dedicated TCP port to spawn a
///   node on (and potentially restart it every now and then).
/// * it is implemented by using SO_REUSEPORT socket option (do not confuse with SO_REUSEADDR),
///   which allows multiple sockets to share a port. reserve_for_test() creates a socket and binds
///   it to a random unused local port (without starting a TCP listener).
///   This socket won't be used for anything but telling the OS that the given TCP port is in use.
///   However thanks to SO_REUSEPORT we can create another socket bind it to the same port
///   and make it a listener.
/// * The reserved port stays reserved until the process terminates - hence during a process
///   lifetime reserve_for_test() should be called a small amount of times (~1000 should be fine,
///   there are only 2^16 ports on a network interface). TODO(gprusak): we may want to track the
///   lifecycle of ListenerAddr (for example via reference counter), so that we can reuse the port
///   after all the references are dropped.
/// * The drawback of this solution that it is hard to detect a situation in which multiple
///   listener sockets in test are bound to the same port. TODO(gprusak): we can prevent creating
///   multiple listeners for a single port within the same process by adding a mutex to the port
///   guard.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ListenerAddr(pub(super) std::net::SocketAddr);

impl fmt::Debug for ListenerAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for ListenerAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::ops::Deref for ListenerAddr {
    type Target = std::net::SocketAddr;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ListenerAddr {
    /// Constructs a new ListenerAddr from std::net::SocketAddr.
    pub fn new(addr: std::net::SocketAddr) -> Self {
        assert!(
            addr.port() != 0,
            "using an anyport (i.e. 0) for the tcp::ListenerAddr is allowed only \
             in tests and only via reserve_for_test() method"
        );
        Self(addr)
    }

    /// Binds a TCP listener to this address.
    pub fn bind(&self) -> std::io::Result<Listener> {
        let socket = match &self.0 {
            std::net::SocketAddr::V4(_) => tokio::net::TcpSocket::new_v4()?,
            std::net::SocketAddr::V6(_) => tokio::net::TcpSocket::new_v6()?,
        };
        if RESERVED_LISTENER_ADDRS
            .lock()
            .unwrap()
            .contains_key(&self.0)
        {
            socket.set_reuseport(true)?;
        }
        socket.set_reuseaddr(true)?;
        socket.bind(self.0)?;
        socket.listen(LISTENER_BACKLOG)
    }
}
