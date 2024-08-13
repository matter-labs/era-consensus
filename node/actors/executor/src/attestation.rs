//! Module to publish attestations over batches.
use anyhow::Context as _;
use std::sync::Arc;
use zksync_concurrency::{ctx, time};
pub use zksync_consensus_network::gossip::attestation::*;
