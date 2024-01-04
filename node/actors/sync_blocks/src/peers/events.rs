//! Events emitted by `PeerStates` actor. Useful for testing.

use zksync_consensus_roles::{node, validator::BlockNumber};

/// Events emitted by `PeerStates` actor. Only used for tests so far.
#[derive(Debug)]
#[allow(dead_code, unused_tuple_struct_fields)] // Variant fields are only read in tests
pub(super) enum PeerStateEvent {
    /// Node has successfully downloaded the specified block.
    GotBlock(BlockNumber),
    /// Received an invalid block from the peer.
    RpcFailed {
        peer_key: node::PublicKey,
        block_number: BlockNumber,
    },
    /// Peer was disconnected (i.e., it has dropped a request).
    PeerDropped(node::PublicKey),
}
