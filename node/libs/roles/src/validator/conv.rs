use super::{
    AggregateSignature, Block, BlockHash, BlockNumber, CommitQC, ConsensusMsg, FinalBlock,
    LeaderCommit, LeaderPrepare, Msg, MsgHash, NetAddress, Phase, PrepareQC, Proposal, PublicKey,
    ReplicaCommit, ReplicaPrepare, Signature, Signed, Signers, ViewNumber,
};
use crate::{node::SessionId, validator};
use ::schema::{read_required, required, ProtoFmt};
use anyhow::Context as _;
use crypto::ByteFmt;
use std::collections::BTreeMap;
use utils::enum_util::Variant;

impl ProtoFmt for BlockHash {
    type Proto = validator::schema::BlockHash;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(ByteFmt::decode(required(&r.sha256)?)?))
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            sha256: Some(self.0.encode()),
        }
    }
}

impl ProtoFmt for Block {
    type Proto = validator::schema::Block;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            parent: read_required(&r.parent).context("parent")?,
            number: BlockNumber(r.number.context("number")?),
            payload: required(&r.payload).context("payload")?.clone(),
        })
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            parent: Some(self.parent.build()),
            number: Some(self.number.0),
            payload: Some(self.payload.clone()),
        }
    }
}

impl ProtoFmt for FinalBlock {
    type Proto = validator::schema::FinalBlock;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            block: read_required(&r.block).context("block")?,
            justification: read_required(&r.justification).context("justification")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            block: Some(self.block.build()),
            justification: Some(self.justification.build()),
        }
    }
}

impl ProtoFmt for ConsensusMsg {
    type Proto = validator::schema::ConsensusMsg;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        use validator::schema::consensus_msg::T;
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
        use validator::schema::consensus_msg::T;

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
    type Proto = validator::schema::ReplicaPrepare;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            view: ViewNumber(r.view.context("view_number")?),
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
    type Proto = validator::schema::ReplicaCommit;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            view: ViewNumber(r.view.context("view_number")?),
            proposal_block_hash: read_required(&r.hash).context("hash")?,
            proposal_block_number: BlockNumber(r.number.context("number")?),
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            view: Some(self.view.0),
            hash: Some(self.proposal_block_hash.build()),
            number: Some(self.proposal_block_number.0),
        }
    }
}

impl ProtoFmt for LeaderPrepare {
    type Proto = validator::schema::LeaderPrepare;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            proposal: read_required(&r.proposal).context("proposal")?,
            justification: read_required(&r.justification).context("justification")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            proposal: Some(self.proposal.build()),
            justification: Some(self.justification.build()),
        }
    }
}

impl ProtoFmt for Proposal {
    type Proto = validator::schema::Proposal;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        use validator::schema::proposal::T;
        Ok(match required(&r.t)? {
            T::New(r) => Self::New(ProtoFmt::read(r).context("Block")?),
            T::Retry(r) => Self::Retry(ProtoFmt::read(r).context("ReplicaCommit")?),
        })
    }

    fn build(&self) -> Self::Proto {
        use validator::schema::proposal::T;
        let t = match self {
            Self::New(x) => T::New(x.build()),
            Self::Retry(x) => T::Retry(x.build()),
        };
        Self::Proto { t: Some(t) }
    }
}

impl ProtoFmt for LeaderCommit {
    type Proto = validator::schema::LeaderCommit;

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
    type Proto = validator::schema::PrepareQc;

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
    type Proto = validator::schema::CommitQc;

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
    type Proto = validator::schema::Phase;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        use validator::schema::phase::T;
        Ok(match required(&r.t)? {
            T::Prepare(_) => Self::Prepare,
            T::Commit(_) => Self::Commit,
        })
    }

    fn build(&self) -> Self::Proto {
        use validator::schema::phase::T;
        let t = match self {
            Self::Prepare => T::Prepare(schema::proto::std::Void {}),
            Self::Commit => T::Commit(schema::proto::std::Void {}),
        };
        Self::Proto { t: Some(t) }
    }
}

impl ProtoFmt for NetAddress {
    type Proto = validator::schema::NetAddress;

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
    type Proto = validator::schema::Msg;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        use validator::schema::msg::T;
        Ok(match r.t.as_ref().context("missing")? {
            T::Consensus(r) => Self::Consensus(ProtoFmt::read(r).context("Consensus")?),
            T::SessionId(r) => Self::SessionId(SessionId(r.clone())),
            T::NetAddress(r) => Self::NetAddress(ProtoFmt::read(r).context("NetAddress")?),
        })
    }

    fn build(&self) -> Self::Proto {
        use validator::schema::msg::T;

        let t = match self {
            Self::Consensus(x) => T::Consensus(x.build()),
            Self::SessionId(x) => T::SessionId(x.0.clone()),
            Self::NetAddress(x) => T::NetAddress(x.build()),
        };

        Self::Proto { t: Some(t) }
    }
}

impl ProtoFmt for MsgHash {
    type Proto = validator::schema::MsgHash;

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
    type Proto = validator::schema::Signed;
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
    type Proto = validator::schema::PublicKey;

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
    type Proto = validator::schema::Signature;

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
    type Proto = validator::schema::AggregateSignature;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(ByteFmt::decode(required(&r.bls12381)?)?))
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            bls12381: Some(self.0.encode()),
        }
    }
}
