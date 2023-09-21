//! Events emitted by `PeerStates` actor. Useful for testing.

use roles::{node, validator::BlockNumber};

/// Events emitted by `PeerStates` actor. Only used for tests so far.
#[derive(Debug)]
#[allow(unused_tuple_struct_fields)] // Variant fields are only read in tests
pub(super) enum PeerStateEvent {
    /// Node has successfully downloaded the specified block.
    GotBlock(BlockNumber),
    /// Peer state was updated. Includes creating a state for a newly connected peer.
    PeerUpdated(node::PublicKey),
    /// Peer was disconnected (i.e., it has dropped a request).
    PeerDisconnected(node::PublicKey),
}
