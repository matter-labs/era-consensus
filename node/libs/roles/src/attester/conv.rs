use crate::proto::attester::{self as proto};
use anyhow::Context;
use zksync_consensus_crypto::ByteFmt;
use zksync_consensus_utils::enum_util::Variant;
use zksync_protobuf::{read_required, required, ProtoFmt};

use super::{L1Batch, Msg, PublicKey, Signature, SignedBatchMsg};

impl ProtoFmt for proto::L1Batch {
    type Proto = proto::L1Batch;

    fn read(_r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {})
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {}
    }
}

impl ProtoFmt for L1Batch {
    type Proto = proto::L1Batch;

    fn read(_r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self)
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {}
    }
}

impl ProtoFmt for SignedBatchMsg {
    type Proto = proto::SignedBatch;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            msg: L1Batch::extract(read_required::<Msg>(&r.msg).context("msg")?)?,
            key: read_required(&r.key).context("key")?,
            sig: read_required(&r.sig).context("sig")?,
        })
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            msg: Some(self.msg.clone().insert().build()),
            key: Some(self.key.build()),
            sig: Some(self.sig.build()),
        }
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

impl ProtoFmt for PublicKey {
    type Proto = proto::PublicKey;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(ByteFmt::decode(required(&r.bn254)?)?))
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            bn254: Some(self.0.encode()),
        }
    }
}

impl ProtoFmt for Signature {
    type Proto = proto::Signature;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(ByteFmt::decode(required(&r.bn254)?)?))
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            bn254: Some(self.0.encode()),
        }
    }
}
