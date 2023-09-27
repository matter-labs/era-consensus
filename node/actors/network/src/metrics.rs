//! General-purpose network metrics.

use crate::state::State;
use std::sync::Weak;
use vise::{Collector, Gauge, Metrics};

/// General-purpose network metrics exposed via a collector.
#[derive(Debug, Metrics)]
#[metrics(prefix = "network")]
pub(crate) struct NetworkGauges {
    /// Number of active inbound gossip connections.
    gossip_inbound_connections: Gauge<usize>,
    /// Number of active outbound gossip connections.
    gossip_outbound_connections: Gauge<usize>,
    /// Number of active inbound consensus connections.
    consensus_inbound_connections: Gauge<usize>,
    /// Number of active outbound consensus connections.
    consensus_outbound_connections: Gauge<usize>,
}

impl NetworkGauges {
    /// Registers a metrics collector for the specified state.
    pub(crate) fn register(state_ref: Weak<State>) {
        #[vise::register]
        static COLLECTOR: Collector<Option<NetworkGauges>> = Collector::new();

        let register_result = COLLECTOR.before_scrape(move || {
            state_ref.upgrade().map(|state| {
                let gauges = NetworkGauges::default();
                let len = state.gossip.inbound.subscribe().borrow().current().len();
                gauges.gossip_inbound_connections.set(len);
                let len = state.gossip.outbound.subscribe().borrow().current().len();
                gauges.gossip_outbound_connections.set(len);
                let len = state.consensus.inbound.subscribe().borrow().current().len();
                gauges.consensus_inbound_connections.set(len);
                let len = state
                    .consensus
                    .outbound
                    .subscribe()
                    .borrow()
                    .current()
                    .len();
                gauges.consensus_outbound_connections.set(len);
                gauges
            })
        });
        if register_result.is_err() {
            tracing::warn!("Failed registering network metrics collector: already registered");
        }
    }
}
