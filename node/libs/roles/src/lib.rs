//! This module provides the roles of the nodes in the network.
//!
//! The roles are:
//! - `Node`: a node that participates in the gossip network, so it receives and broadcast blocks,
//!           helps with peer discovery, etc. Every node has this role.
//! - `Validator`: a node that participates in the consensus protocol, so it votes for blocks and produces blocks.
//!                It also participates in the validator network, which is a mesh network just for validators. Not
//!                every node has this role.
//! - `Attester`: a node that signs the L1 batches and broadcasts the signatures to the gossip network.
//!                Not every node has this role.

pub mod attester;
pub mod node;
pub mod proto;
pub mod validator;
