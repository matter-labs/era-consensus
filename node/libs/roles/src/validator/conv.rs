use super::*;
use crate::{node::SessionId};
use ::schema::{read_required, required, ProtoFmt};
use anyhow::Context as _;
use crypto::ByteFmt;
use std::collections::BTreeMap;
use utils::enum_util::Variant;
use schema::proto::roles::validator as proto;

impl ProtoFmt for BlockHeaderHash {
    type Proto = proto::BlockHeaderHash;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(ByteFmt::decode(required(&r.sha256)?)?))
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            sha256: Some(self.0.encode()),
        }
    }
}

impl ProtoFmt for PayloadHash {
    type Proto = proto::PayloadHash;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(ByteFmt::decode(required(&r.sha256)?)?))
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            sha256: Some(self.0.encode()),
        }
    }
}

impl ProtoFmt for BlockHeader {
    type Proto = proto::BlockHeader;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            protocol_version: ProtocolVersion(r.protocol_version.context("protocol_version")?),
            parent: read_required(&r.parent).context("parent")?,
            number: BlockNumber(r.number.context("number")?),
            payload: read_required(&r.payload).context("payload")?,
        })
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            protocol_version: Some(self.protocol_version.0),
            parent: Some(self.parent.build()),
            number: Some(self.number.0),
            payload: Some(self.payload.build()),
        }
    }
}

impl ProtoFmt for FinalBlock {
    type Proto = proto::FinalBlock;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            header: read_required(&r.header).context("header")?,
            payload: Payload(required(&r.payload).context("payload")?.clone()),
            justification: read_required(&r.justification).context("justification")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            header: Some(self.header.build()),
            payload: Some(self.payload.0.clone()),
            justification: Some(self.justification.build()),
        }
    }
}

impl ProtoFmt for ConsensusMsg {
    type Proto = proto::ConsensusMsg;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        use proto::consensus_msg::T;
        Ok(match r.t.as_ref().context("missing")? {
            T::ReplicaPrepare(r) => {
                Self::ReplicaPrepare(ProtoFmt::read(r).context("ReplicaPrepare")?)
            }
            T::ReplicaCommit(r) => Self::ReplicaCommit(ProtoFmt::read(r).context("ReplicaCommit")?),
            T::LeaderPrepare(r) => Self::LeaderPrepare(ProtoFmt::read(r).context("LeaderPrepare")?),
            T::LeaderCommit(r) => Self::LeaderCommit(ProtoFmt::read(r).context("LeaderCommit")?),
        })
    }

    fn build(&self) -> Self::Proto {
        use proto::consensus_msg::T;

        let t = match self {
            Self::ReplicaPrepare(x) => T::ReplicaPrepare(x.build()),
            Self::ReplicaCommit(x) => T::ReplicaCommit(x.build()),
            Self::LeaderPrepare(x) => T::LeaderPrepare(x.build()),
            Self::LeaderCommit(x) => T::LeaderCommit(x.build()),
        };

        Self::Proto { t: Some(t) }
    }
}

impl ProtoFmt for ReplicaPrepare {
    type Proto = proto::ReplicaPrepare;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            view: ViewNumber(*required(&r.view).context("view")?),
            high_vote: read_required(&r.high_vote).context("high_vote")?,
            high_qc: read_required(&r.high_qc).context("high_qc")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            view: Some(self.view.0),
            high_vote: Some(self.high_vote.build()),
            high_qc: Some(self.high_qc.build()),
        }
    }
}

impl ProtoFmt for ReplicaCommit {
    type Proto = proto::ReplicaCommit;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            view: ViewNumber(*required(&r.view).context("view")?),
            proposal: read_required(&r.proposal).context("proposal")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            view: Some(self.view.0),
            proposal: Some(self.proposal.build()),
        }
    }
}

impl ProtoFmt for LeaderPrepare {
    type Proto = proto::LeaderPrepare;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            view: ViewNumber(*required(&r.view).context("view")?),
            proposal: read_required(&r.proposal).context("proposal")?,
            proposal_payload: r.proposal_payload.as_ref().map(|p|Payload(p.clone())),
            justification: read_required(&r.justification).context("justification")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            view: Some(self.view.0),
            proposal: Some(self.proposal.build()),
            proposal_payload: self.proposal_payload.as_ref().map(|p|p.0.clone()),
            justification: Some(self.justification.build()),
        }
    }
}

impl ProtoFmt for LeaderCommit {
    type Proto = proto::LeaderCommit;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            justification: read_required(&r.justification).context("justification")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            justification: Some(self.justification.build()),
        }
    }
}

impl ProtoFmt for PrepareQC {
    type Proto = proto::PrepareQc;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        let mut map = BTreeMap::new();

        for (msg, signers) in r.msgs.iter().zip(r.signers.iter()) {
            map.insert(
                read_required::<ReplicaPrepare>(&Some(msg).cloned()).context("msg")?,
                Signers::decode(signers).context("signers")?,
            );
        }

        Ok(Self {
            map,
            signature: read_required(&r.sig).context("sig")?,
        })
    }

    fn build(&self) -> Self::Proto {
        let (msgs, signers) = self
            .map
            .iter()
            .map(|(msg, signers)| (msg.clone().build(), signers.encode()))
            .unzip();

        Self::Proto {
            msgs,
            signers,
            sig: Some(self.signature.build()),
        }
    }
}

impl ProtoFmt for CommitQC {
    type Proto = proto::CommitQc;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            message: read_required(&r.msg).context("msg")?,
            signers: ByteFmt::decode(required(&r.signers).context("signers")?)?,
            signature: read_required(&r.sig).context("sig")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            msg: Some(self.message.build()),
            signers: Some(self.signers.encode()),
            sig: Some(self.signature.build()),
        }
    }
}

impl ProtoFmt for Phase {
    type Proto = proto::Phase;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        use proto::phase::T;
        Ok(match required(&r.t)? {
            T::Prepare(_) => Self::Prepare,
            T::Commit(_) => Self::Commit,
        })
    }

    fn build(&self) -> Self::Proto {
        use proto::phase::T;
        let t = match self {
            Self::Prepare => T::Prepare(schema::proto::std::Void {}),
            Self::Commit => T::Commit(schema::proto::std::Void {}),
        };
        Self::Proto { t: Some(t) }
    }
}

impl ProtoFmt for NetAddress {
    type Proto = proto::NetAddress;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            addr: read_required(&r.addr).context("addr")?,
            version: *required(&r.version).context("version")?,
            timestamp: read_required(&r.timestamp).context("timestamp")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            addr: Some(self.addr.build()),
            version: Some(self.version),
            timestamp: Some(self.timestamp.build()),
        }
    }
}

impl ProtoFmt for Msg {
    type Proto = proto::Msg;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        use proto::msg::T;
        Ok(match r.t.as_ref().context("missing")? {
            T::Consensus(r) => Self::Consensus(ProtoFmt::read(r).context("Consensus")?),
            T::SessionId(r) => Self::SessionId(SessionId(r.clone())),
            T::NetAddress(r) => Self::NetAddress(ProtoFmt::read(r).context("NetAddress")?),
        })
    }

    fn build(&self) -> Self::Proto {
        use proto::msg::T;

        let t = match self {
            Self::Consensus(x) => T::Consensus(x.build()),
            Self::SessionId(x) => T::SessionId(x.0.clone()),
            Self::NetAddress(x) => T::NetAddress(x.build()),
        };

        Self::Proto { t: Some(t) }
    }
}

impl ProtoFmt for MsgHash {
    type Proto = proto::MsgHash;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(ByteFmt::decode(required(&r.sha256)?)?))
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            sha256: Some(self.0.encode()),
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

impl ProtoFmt for PublicKey {
    type Proto = proto::PublicKey;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(ByteFmt::decode(required(&r.bls12381)?)?))
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            bls12381: Some(self.0.encode()),
        }
    }
}

impl ProtoFmt for Signature {
    type Proto = proto::Signature;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(ByteFmt::decode(required(&r.bls12381)?)?))
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            bls12381: Some(self.0.encode()),
        }
    }
}

impl ProtoFmt for AggregateSignature {
    type Proto = proto::AggregateSignature;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(ByteFmt::decode(required(&r.bls12381)?)?))
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            bls12381: Some(self.0.encode()),
        }
    }
}
