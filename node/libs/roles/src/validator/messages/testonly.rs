use rand::{
    distributions::{Distribution, Standard},
    Rng,
};
use zksync_concurrency::time;
use zksync_consensus_utils::enum_util::Variant;

use super::{
    v1, Block, BlockNumber, ChainId, ConsensusMsg, ForkNumber, Genesis, GenesisHash, GenesisRaw,
    Justification, Msg, MsgHash, NetAddress, Payload, PayloadHash, PreGenesisBlock, Proposal,
    ProtocolVersion, ReplicaState, Signed, ViewNumber,
};
use crate::validator::SecretKey;

impl Distribution<ViewNumber> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ViewNumber {
        ViewNumber(rng.gen())
    }
}

impl Distribution<BlockNumber> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> BlockNumber {
        BlockNumber(rng.gen())
    }
}

impl Distribution<ProtocolVersion> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ProtocolVersion {
        ProtocolVersion(rng.gen())
    }
}

impl Distribution<ForkNumber> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ForkNumber {
        ForkNumber(rng.gen())
    }
}

impl Distribution<ChainId> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ChainId {
        ChainId(rng.gen())
    }
}

impl Distribution<GenesisHash> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> GenesisHash {
        GenesisHash(rng.gen())
    }
}

impl Distribution<GenesisRaw> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> GenesisRaw {
        let mut genesis = GenesisRaw {
            chain_id: rng.gen(),
            fork_number: rng.gen(),
            first_block: rng.gen(),

            protocol_version: rng.gen(),
            validators: rng.gen(),
            leader_selection: rng.gen(),
        };

        // In order for the genesis to be valid, sticky/rota leaders need to be in the validator committee.
        if let v1::LeaderSelectionMode::Sticky(_) = genesis.leader_selection {
            let i = rng.gen_range(0..genesis.validators.len());
            genesis.leader_selection =
                v1::LeaderSelectionMode::Sticky(genesis.validators.get(i).unwrap().key.clone());
        } else if let v1::LeaderSelectionMode::Rota(pks) = genesis.leader_selection {
            let n = pks.len();
            let mut pks = Vec::new();
            for _ in 0..n {
                let i = rng.gen_range(0..genesis.validators.len());
                pks.push(genesis.validators.get(i).unwrap().key.clone());
            }
            genesis.leader_selection = v1::LeaderSelectionMode::Rota(pks);
        }

        genesis
    }
}

impl Distribution<Genesis> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Genesis {
        rng.gen::<GenesisRaw>().with_hash()
    }
}

impl Distribution<PayloadHash> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> PayloadHash {
        PayloadHash(rng.gen())
    }
}

impl Distribution<Payload> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Payload {
        let size: usize = rng.gen_range(500..1000);
        Payload((0..size).map(|_| rng.gen()).collect())
    }
}

impl Distribution<Proposal> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Proposal {
        Proposal {
            number: rng.gen(),
            payload: rng.gen(),
        }
    }
}

impl Distribution<Justification> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Justification {
        let size: usize = rng.gen_range(500..1000);
        Justification((0..size).map(|_| rng.gen()).collect())
    }
}

impl Distribution<PreGenesisBlock> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> PreGenesisBlock {
        PreGenesisBlock {
            number: rng.gen(),
            payload: rng.gen(),
            justification: rng.gen(),
        }
    }
}

impl Distribution<Block> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Block {
        match rng.gen_range(0..2) {
            0 => Block::PreGenesis(rng.gen()),
            _ => Block::FinalV1(rng.gen()),
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

impl Distribution<ConsensusMsg> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ConsensusMsg {
        match rng.gen_range(0..4) {
            0 => ConsensusMsg::LeaderProposal(rng.gen()),
            1 => ConsensusMsg::ReplicaCommit(rng.gen()),
            2 => ConsensusMsg::ReplicaNewView(rng.gen()),
            3 => ConsensusMsg::ReplicaTimeout(rng.gen()),
            _ => unreachable!(),
        }
    }
}

impl Distribution<ReplicaState> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ReplicaState {
        ReplicaState {
            view: rng.gen(),
            phase: rng.gen(),
            high_vote: rng.gen(),
            high_commit_qc: rng.gen(),
            high_timeout_qc: rng.gen(),
            proposals: (0..rng.gen_range(1..11)).map(|_| rng.gen()).collect(),
            v2: rng.gen(),
        }
    }
}
