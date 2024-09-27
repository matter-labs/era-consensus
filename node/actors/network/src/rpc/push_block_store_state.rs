//! RPC for notifying peer about our BlockStore state.
use super::Capability;
use crate::proto::gossip as proto;
use anyhow::Context;
use zksync_consensus_roles::validator;
use zksync_consensus_storage::{BlockStoreState, Last};
use zksync_protobuf::{read_optional, read_optional_repr, required, ProtoFmt, ProtoRepr};

/// PushBlockStoreState RPC.
#[derive(Debug)]
pub(crate) struct Rpc;

impl super::Rpc for Rpc {
    const CAPABILITY: Capability = Capability::PushBlockStoreState;
    const INFLIGHT: u32 = 1;
    const METHOD: &'static str = "push_block_store_state";

    type Req = Req;
    type Resp = ();
}

/// Contains the freshest state of the sender's block store.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Req {
    // DEPRECATED: transmitted for backward compatibility.
    first: validator::BlockNumber,
    // DEPRECATED: transmitted for backward compatibility.
    last: Option<validator::CommitQC>,
    // Block store state. Will be required once we drop
    // compatibility for `first` and `last` fields.
    state: Option<BlockStoreState>,
}

impl Req {
    /// Constructs a new request.
    pub(crate) fn new(state: BlockStoreState, genesis: &validator::Genesis) -> Self {
        Req {
            first: state.first.max(genesis.first_block),
            last: match &state.last {
                Some(Last::Final(qc)) => Some(qc.clone()),
                _ => None,
            },
            state: Some(state),
        }
    }

    /// Extracts block store state from the request.
    pub(crate) fn state(&self) -> BlockStoreState {
        match &self.state {
            Some(state) => state.clone(),
            None => BlockStoreState {
                first: self.first,
                last: self.last.clone().map(Last::Final),
            },
        }
    }
}

impl ProtoRepr for proto::Last {
    type Type = Last;
    fn read(&self) -> anyhow::Result<Self::Type> {
        use proto::last::T;
        Ok(match self.t.as_ref().context("missing")? {
            T::PreGenesis(n) => Last::PreGenesis(validator::BlockNumber(*n)),
            T::Final(qc) => Last::Final(ProtoFmt::read(qc).context("final")?),
        })
    }
    fn build(this: &Self::Type) -> Self {
        use proto::last::T;
        Self {
            t: Some(match this {
                Last::PreGenesis(n) => T::PreGenesis(n.0),
                Last::Final(qc) => T::Final(qc.build()),
            }),
        }
    }
}

impl ProtoRepr for proto::BlockStoreState {
    type Type = BlockStoreState;
    fn read(&self) -> anyhow::Result<Self::Type> {
        Ok(Self::Type {
            first: validator::BlockNumber(*required(&self.first).context("first")?),
            last: read_optional_repr(&self.last).context("last")?,
        })
    }
    fn build(this: &Self::Type) -> Self {
        Self {
            first: Some(this.first.0),
            last: this.last.as_ref().map(ProtoRepr::build),
        }
    }
}

impl ProtoFmt for Req {
    type Proto = proto::PushBlockStoreState;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        let state: Option<BlockStoreState> = read_optional_repr(&r.state).context("state")?;
        Ok(Self {
            first: r
                .first
                .map(validator::BlockNumber)
                .or(state.as_ref().map(|s| s.first))
                .context("missing first and state")?,
            last: read_optional(&r.last).context("last")?,
            state,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            first: Some(self.first.0),
            last: self.last.as_ref().map(|x| x.build()),
            state: self.state.as_ref().map(ProtoRepr::build),
        }
    }
}
