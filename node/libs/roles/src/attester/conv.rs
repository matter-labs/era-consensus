use crate::proto::attester::{self as proto};
use anyhow::Context;
use zksync_consensus_crypto::ByteFmt;
use zksync_consensus_utils::enum_util::Variant;
use zksync_protobuf::{read_required, required, ProtoFmt};

use super::{
    AggregateSignature, BatchNumber, L1Batch, L1BatchQC, Msg, MsgHash, PublicKey, Signature,
    SignedBatchMsg, Signers, WeightedAttester,
};

impl ProtoFmt for L1Batch {
    type Proto = proto::L1Batch;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            number: BatchNumber(*required(&r.number).context("number")?),
        })
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            number: Some(self.number.0),
        }
    }
}

impl<V: Variant<Msg> + Clone> ProtoFmt for SignedBatchMsg<V> {
    type Proto = proto::SignedBatch;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            msg: V::extract(read_required::<Msg>(&r.msg).context("msg")?)?,
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

impl ProtoFmt for WeightedAttester {
    type Proto = proto::WeightedAttester;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            key: read_required(&r.key).context("key")?,
            weight: *required(&r.weight).context("weight")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            key: Some(self.key.build()),
            weight: Some(self.weight),
        }
    }
}

impl ProtoFmt for Signers {
    type Proto = zksync_protobuf::proto::std::BitVector;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(ProtoFmt::read(r)?))
    }

    fn build(&self) -> Self::Proto {
        self.0.build()
    }
}

impl ProtoFmt for AggregateSignature {
    type Proto = proto::AggregateSignature;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(ByteFmt::decode(required(&r.bn254)?)?))
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            bn254: Some(self.0.encode()),
        }
    }
}

impl ProtoFmt for MsgHash {
    type Proto = proto::MsgHash;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(ByteFmt::decode(required(&r.keccak256)?)?))
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            keccak256: Some(self.0.encode()),
        }
    }
}

impl ProtoFmt for L1BatchQC {
    type Proto = proto::L1BatchQc;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            message: read_required(&r.msg).context("message")?,
            signers: read_required(&r.signers).context("signers")?,
            signature: read_required(&r.sig).context("signature")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            msg: Some(self.message.build()),
            signers: Some(self.signers.build()),
            sig: Some(self.signature.build()),
        }
    }
}
