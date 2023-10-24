use super::*;
use bit_vec::BitVec;
use concurrency::time;
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};
use std::sync::Arc;
use utils::enum_util::Variant;

impl Distribution<AggregateSignature> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> AggregateSignature {
        AggregateSignature(rng.gen())
    }
}

impl Distribution<PublicKey> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> PublicKey {
        PublicKey(rng.gen())
    }
}

impl Distribution<Signature> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Signature {
        Signature(rng.gen())
    }
}

impl Distribution<SecretKey> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> SecretKey {
        SecretKey(Arc::new(rng.gen()))
    }
}

impl Distribution<BlockNumber> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> BlockNumber {
        BlockNumber(rng.gen())
    }
}

impl Distribution<PayloadHash> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> PayloadHash {
        PayloadHash(rng.gen())
    }
}

impl Distribution<BlockHeaderHash> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> BlockHeaderHash {
        BlockHeaderHash(rng.gen())
    }
}

impl Distribution<BlockHeader> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> BlockHeader {
        BlockHeader {
            protocol_version: ProtocolVersion(rng.gen()),
            parent: rng.gen(),
            number: rng.gen(),
            payload: rng.gen(),
        }
    }
}

impl Distribution<Payload> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Payload {
        let size: usize = rng.gen_range(0..11);
        Payload((0..size).map(|_| rng.gen()).collect())
    }
}

impl Distribution<FinalBlock> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> FinalBlock {
        FinalBlock {
            header: rng.gen(),
            payload: rng.gen(),
            justification: rng.gen(),
        }
    }
}

impl Distribution<ReplicaPrepare> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ReplicaPrepare {
        ReplicaPrepare {
            view: rng.gen(),
            high_vote: rng.gen(),
            high_qc: rng.gen(),
        }
    }
}

impl Distribution<ReplicaCommit> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ReplicaCommit {
        ReplicaCommit {
            view: rng.gen(),
            proposal: rng.gen(),
        }
    }
}

impl Distribution<LeaderPrepare> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> LeaderPrepare {
        LeaderPrepare {
            view: rng.gen(),
            proposal: rng.gen(),
            proposal_payload: rng.gen(),
            justification: rng.gen(),
        }
    }
}

impl Distribution<LeaderCommit> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> LeaderCommit {
        LeaderCommit {
            justification: rng.gen(),
        }
    }
}

impl Distribution<PrepareQC> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> PrepareQC {
        let n = rng.gen_range(1..11);
        let map = (0..n).map(|_| (rng.gen(), rng.gen())).collect();

        PrepareQC {
            map,
            signature: rng.gen(),
        }
    }
}

impl Distribution<CommitQC> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> CommitQC {
        CommitQC {
            message: rng.gen(),
            signers: rng.gen::<Signers>(),
            signature: rng.gen::<AggregateSignature>(),
        }
    }
}

impl Distribution<Signers> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Signers {
        Signers(BitVec::from_bytes(&rng.gen::<[u8; 4]>()))
    }
}

impl Distribution<ValidatorSet> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ValidatorSet {
        let count = rng.gen_range(1..11);
        let public_keys = (0..count).map(|_| rng.gen());
        ValidatorSet::new(public_keys).unwrap()
    }
}

impl Distribution<ViewNumber> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ViewNumber {
        ViewNumber(rng.gen())
    }
}

impl Distribution<Phase> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Phase {
        let i = rng.gen_range(0..2);

        match i {
            0 => Phase::Prepare,
            1 => Phase::Commit,
            _ => unreachable!(),
        }
    }
}

impl Distribution<NetAddress> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> NetAddress {
        NetAddress {
            addr: std::net::SocketAddr::new(
                std::net::IpAddr::from(rng.gen::<[u8; 16]>()),
                rng.gen(),
            ),
            version: rng.gen(),
            timestamp: time::UNIX_EPOCH + time::Duration::seconds(rng.gen_range(0..1000000000)),
        }
    }
}

impl Distribution<Msg> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Msg {
        match rng.gen_range(0..3) {
            0 => Msg::Consensus(rng.gen()),
            1 => Msg::SessionId(rng.gen()),
            2 => Msg::NetAddress(rng.gen()),
            _ => unreachable!(),
        }
    }
}

impl Distribution<ConsensusMsg> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ConsensusMsg {
        match rng.gen_range(0..4) {
            0 => ConsensusMsg::ReplicaPrepare(rng.gen()),
            1 => ConsensusMsg::ReplicaCommit(rng.gen()),
            2 => ConsensusMsg::LeaderPrepare(rng.gen()),
            3 => ConsensusMsg::LeaderCommit(rng.gen()),
            _ => unreachable!(),
        }
    }
}

impl Distribution<MsgHash> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> MsgHash {
        MsgHash(rng.gen())
    }
}

impl<V: Variant<Msg>> Distribution<Signed<V>> for Standard
where
    Standard: Distribution<V>,
{
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Signed<V> {
        rng.gen::<SecretKey>().sign_msg(rng.gen())
    }
}
