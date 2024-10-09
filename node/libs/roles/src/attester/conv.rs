use super::{
    AggregateSignature, Batch, BatchHash, BatchNumber, BatchQC, Msg, MsgHash, MultiSig, PublicKey,
    Signature, Signed, Signers, WeightedAttester,
};
use crate::proto::attester as proto;
use anyhow::Context as _;
use zksync_consensus_crypto::ByteFmt;
use zksync_consensus_utils::enum_util::Variant;
use zksync_protobuf::{read_map, read_required, required, ProtoFmt};

impl ProtoFmt for BatchHash {
    type Proto = proto::BatchHash;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(ByteFmt::decode(required(&r.keccak256)?)?))
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            keccak256: Some(self.0.encode()),
        }
    }
}

impl ProtoFmt for Batch {
    type Proto = proto::Batch;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            number: BatchNumber(*required(&r.number).context("number")?),
            hash: read_required(&r.hash).context("hash")?,
            genesis: read_required(&r.genesis).context("genesis")?,
        })
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            number: Some(self.number.0),
            hash: Some(self.hash.build()),
            genesis: Some(self.genesis.build()),
        }
    }
}

impl<V: Variant<Msg> + Clone> ProtoFmt for Signed<V> {
    type Proto = proto::Signed;
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
            T::Batch(r) => Self::Batch(ProtoFmt::read(r).context("Batch")?),
        })
    }

    fn build(&self) -> Self::Proto {
        use proto::msg::T;

        let t = match self {
            Self::Batch(x) => T::Batch(x.build()),
        };

        Self::Proto { t: Some(t) }
    }
}

impl ProtoFmt for PublicKey {
    type Proto = proto::PublicKey;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(ByteFmt::decode(required(&r.secp256k1)?)?))
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            secp256k1: Some(self.0.encode()),
        }
    }
}

impl ProtoFmt for Signature {
    type Proto = proto::Signature;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(ByteFmt::decode(required(&r.secp256k1)?)?))
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            secp256k1: Some(self.0.encode()),
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
        Ok(Self(ByteFmt::decode(required(&r.bls12_381)?)?))
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            bls12_381: Some(self.0.encode()),
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

impl ProtoFmt for BatchQC {
    type Proto = proto::BatchQc;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            message: read_required(&r.msg).context("message")?,
            signatures: MultiSig(
                read_map(&r.signatures, |s| &s.key, |s| &s.sig).context("signatures")?,
            ),
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            msg: Some(self.message.build()),
            signatures: self
                .signatures
                .iter()
                .map(|(pk, sig)| proto::Attestation {
                    key: Some(pk.build()),
                    sig: Some(sig.build()),
                })
                .collect(),
        }
    }
}
