use crate::ConsensusInner;
use concurrency::{ctx, time};
use once_cell::sync::Lazy;
use roles::validator;
use std::{
    collections::{BTreeMap, HashMap},
    unreachable,
};
use tracing::{instrument, warn};

static PROCESSING_LATENCY: Lazy<prometheus::HistogramVec> = Lazy::new(|| {
    prometheus::register_histogram_vec!(
        "consensus_leader__processing_latency",
        "latency of processing messages",
        &["type", "result"]
    )
    .unwrap()
});

fn result_label<T, E>(res: &Result<T, E>) -> &str {
    match res {
        Ok(_) => "ok",
        Err(_) => "err",
    }
}

/// The StateMachine struct contains the state of the leader. This is a simple state machine. We just store
/// replica messages and produce leader messages (including proposing blocks) when we reach the threshold for
/// those messages. When participating in consensus we are not the leader most of the time.
#[derive(Debug)]
pub(crate) struct StateMachine {
    /// The current view number. This might not match the replica's view number, we only have this here
    /// to make the leader advance monotonically in time and stop it from accepting messages from the past.
    pub(crate) view: validator::ViewNumber,
    /// The current phase. This might not match the replica's phase, we only have this here
    /// to make the leader advance monotonically in time and stop it from accepting messages from the past.
    pub(crate) phase: validator::Phase,
    /// Time when the current phase has started.
    pub(crate) phase_start: time::Instant,
    /// A cache of our latest block proposal. We use it to determine if we accept a replica commit message.
    pub(crate) block_proposal_cache: Option<validator::ReplicaCommit>,
    /// A cache of replica prepare messages indexed by view number and validator.
    pub(crate) prepare_message_cache: BTreeMap<
        validator::ViewNumber,
        HashMap<validator::PublicKey, validator::Signed<validator::ReplicaPrepare>>,
    >,
    /// A cache of replica commit messages indexed by view number and validator.
    pub(crate) commit_message_cache: BTreeMap<
        validator::ViewNumber,
        HashMap<validator::PublicKey, validator::Signed<validator::ReplicaCommit>>,
    >,
}

impl StateMachine {
    /// Creates a new StateMachine struct.
    #[instrument(level = "trace", ret)]
    pub fn new(ctx: &ctx::Ctx) -> Self {
        StateMachine {
            view: validator::ViewNumber(0),
            phase: validator::Phase::Prepare,
            phase_start: ctx.now(),
            block_proposal_cache: None,
            prepare_message_cache: BTreeMap::new(),
            commit_message_cache: BTreeMap::new(),
        }
    }

    /// Process an input message (leaders don't time out waiting for a message). This is the
    /// main entry point for the state machine. We need read-access to the inner consensus struct.
    /// As a result, we can modify our state machine or send a message to the executor.  
    #[instrument(level = "trace", ret)]
    pub(crate) fn process_input(
        &mut self,
        ctx: &ctx::Ctx,
        consensus: &ConsensusInner,
        input: validator::Signed<validator::ConsensusMsg>,
    ) {
        let now = ctx.now();
        let (type_, result) = match &input.msg {
            validator::ConsensusMsg::ReplicaPrepare(_) => (
                "ReplicaPrepare",
                self.process_replica_prepare(ctx, consensus, input.cast().unwrap()),
            ),
            validator::ConsensusMsg::ReplicaCommit(_) => (
                "ReplicaCommit",
                self.process_replica_commit(ctx, consensus, input.cast().unwrap()),
            ),
            _ => unreachable!(),
        };
        PROCESSING_LATENCY
            .with_label_values(&[type_, result_label(&result)])
            .observe((ctx.now() - now).as_seconds_f64());
        if let Err(e) = result {
            warn!("{}", e);
        }
    }
}
