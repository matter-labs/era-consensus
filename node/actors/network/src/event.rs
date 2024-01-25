//! Mechanism for network State to report internal events.
//! It is used in tests to await a specific state.
use crate::State;

impl State {
    /// Sends an event to the `self.events` channel.
    /// Noop if `self.events` is None.
    pub(crate) fn event(&self, e: Event) {
        if let Some(events) = &self.events {
            events.send(e);
        }
    }
}

/// Events observable in tests.
/// Feel free to extend this enum if you need to
/// write a test awaiting some specific event/state.
#[derive(Debug)]
pub enum Event {
    ValidatorAddrsUpdated,
}
