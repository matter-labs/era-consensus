use std::sync::Arc;

use super::{rng, Clock, Ctx, Inner};
use crate::{signal, time};

/// Returns a root context with the given `clock`
/// and a deterministic RNG provider.
pub fn test_root<C: Clone + Into<Clock>>(clock: &C) -> Ctx {
    Ctx(Arc::new(Inner {
        clock: clock.clone().into(),
        rng_provider: rng::Provider::test(),
        canceled: Arc::new(signal::Once::new()),
        deadline: time::Deadline::Infinite,
        _parent: None,
    }))
}

/// Returns a child context with the given clock.
/// Useful for simulating entities with independent clocks.
pub fn test_with_clock<C: Clone + Into<Clock>>(ctx: &Ctx, clock: &C) -> Ctx {
    ctx.child_with_clock(clock.clone().into(), ctx.0.deadline)
}
