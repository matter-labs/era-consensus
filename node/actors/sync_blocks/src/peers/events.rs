//! Events emitted by `PeerStates` actor. Useful for testing.

use zksync_consensus_roles::{node, validator::BlockNumber};

/// Events emitted by `PeerStates` actor. Only used for tests so far.
#[derive(Debug)]
#[allow(dead_code, unused_tuple_struct_fields)] // Variant fields are only read in tests
pub(super) enum PeerStateEvent {
    /// Node has successfully downloaded the specified block.
    GotBlock(BlockNumber),
    /// Block retrieval was canceled due to block getting persisted using other means.
    CanceledBlock(BlockNumber),
    /// Received an invalid block from the peer.
    GotInvalidBlock {
        peer_key: node::PublicKey,
        block_number: BlockNumber,
    },
    /// Peer state was updated. Includes creating a state for a newly connected peer.
    PeerUpdated(node::PublicKey),
    /// Received invalid `SyncState` from a peer.
    InvalidPeerUpdate(node::PublicKey),
    /// Peer was disconnected (i.e., it has dropped a request).
    PeerDisconnected(node::PublicKey),
}
