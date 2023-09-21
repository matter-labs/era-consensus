//! Network actor maintaining a pool of outbound and inbound connections to other nodes.

// &*x is not equivalent to x, because it affects borrowing in closures.
#![allow(clippy::borrow_deref_ref)]

pub use state::*;
pub mod consensus;
mod event;
mod frame;
pub mod gossip;
pub mod io;
mod mux;
mod noise;
mod pool;
mod preface;
mod rpc;
mod state;
pub mod testonly;
#[cfg(test)]
mod tests;
mod watch;
