//! Global state distributed by active attesters, observed by all the nodes in the network.
use super::metrics;
use crate::watch::Watch;
use crate::gossip::AttestationStatus;
use std::cmp::Ordering;
use std::{collections::HashSet, fmt, sync::Arc};
use zksync_concurrency::sync;
use zksync_consensus_roles::attester;

/// Watch wrapper of BatchVotes,
/// which supports subscribing to BatchVotes updates.
pub(crate) struct BatchVotesWatch(Watch<BatchVotes>);
