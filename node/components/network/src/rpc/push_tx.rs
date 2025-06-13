//! RPC for propagating user transactions.
use anyhow::Context as _;
use zksync_consensus_engine::Transaction;
use zksync_protobuf::{read_required_repr, required, ProtoFmt, ProtoRepr};

use super::Capability;
use crate::proto::gossip as proto;

/// PushTx RPC.
pub(crate) struct Rpc;

impl super::Rpc for Rpc {
    const CAPABILITY: Capability = Capability::PushTx;
    const INFLIGHT: u32 = 1;
    const METHOD: &'static str = "push_tx";

    type Req = Req;
    type Resp = ();
}

/// Contains a new Transaction that the sender has learned about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Req(pub(crate) Transaction);

impl ProtoFmt for Req {
    type Proto = proto::PushTx;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(read_required_repr(&r.tx)?))
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            tx: Some(ProtoRepr::build(&self.0)),
        }
    }
}

impl ProtoRepr for proto::Transaction {
    type Type = Transaction;
    fn read(&self) -> anyhow::Result<Self::Type> {
        Ok(Transaction(required(&self.tx).context("tx")?.clone()))
    }
    fn build(this: &Self::Type) -> Self {
        Self {
            tx: Some(this.0.clone()),
        }
    }
}
