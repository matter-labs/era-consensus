use crate::proto::attester::{self as proto, SignedBatch};
use anyhow::Context;
use zksync_consensus_crypto::ByteFmt;
use zksync_consensus_utils::enum_util::Variant;
use zksync_protobuf::{read_required, required, ProtoFmt};

use super::{BatchPublicKey, BatchSignature, L1BatchMsg, Msg};

impl ProtoFmt for proto::L1BatchMsg {
    type Proto = proto::L1BatchMsg;

    fn read(_r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {})
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {}
    }
}

impl ProtoFmt for L1BatchMsg {
    type Proto = proto::L1BatchMsg;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self())
    }

    fn build(&self) -> Self::Proto {
        self.build()
    }
}

impl ProtoFmt for Msg {
    type Proto = proto::Msg;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        use proto::msg::T;
        Ok(match r.t.as_ref().context("missing")? {
            T::L1Batch(r) => Self::L1Batch(ProtoFmt::read(r).context("L1Batch")?),
        })
    }

    fn build(&self) -> Self::Proto {
        use proto::msg::T;

        let t = match self {
            Self::L1Batch(x) => T::L1Batch(x.build()),
        };

        Self::Proto { t: Some(t) }
    }
}

impl ProtoFmt for BatchPublicKey {
    type Proto = proto::BatchPublicKey;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(ByteFmt::decode(required(&r.bn254)?)?))
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            bn254: Some(self.0.encode()),
        }
    }
}

impl ProtoFmt for BatchSignature {
    type Proto = proto::BatchSignature;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(ByteFmt::decode(required(&r.bn254)?)?))
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            bn254: Some(self.0.encode()),
        }
    }
}
