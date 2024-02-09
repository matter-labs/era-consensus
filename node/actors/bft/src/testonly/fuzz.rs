use crate::testonly::node::MAX_PAYLOAD_SIZE;
use rand::{seq::SliceRandom, Rng, distributions::{Distribution, Standard}};
use zksync_consensus_roles::validator;

/// Trait that allows us to mutate types. It's an approach to fuzzing that instead of starting with completely random inputs
/// (which will basically always fail on the first check), starts from a real valid message and modifies a single value to
/// make it invalid. The idea is to try to hit as many checks as possible, and create more realistic malicious messages.
pub(crate) trait Fuzz {
    /// Mutates a message. It will take a message (ideally a valid message, but we don't check this here) and change a single
    /// value to make it invalid (but ideally pretty close to being valid).
    fn mutate(&mut self, rng: &mut impl Rng);
}

impl<T:Fuzz> Fuzz for Option<T> where Standard: Distribution<T> {
    fn mutate(&mut self, rng: &mut impl Rng) {
        if let Some(v) = self.as_mut() {
            v.mutate(rng);
        } else {
            *self = Some(rng.gen());
        }
    }
}

impl Fuzz for validator::Signed<validator::ConsensusMsg> {
    fn mutate(&mut self, rng: &mut impl Rng) {
        // We give them different weights because we want to mutate the message more often.
        match rng.gen_range(0..20) {
            0..=17 => self.msg.mutate(rng),
            18 => self.key = rng.gen(),
            19 => self.sig = rng.gen(),
            _ => unreachable!(),
        }
    }
}

impl Fuzz for validator::ConsensusMsg {
    fn mutate(&mut self, rng: &mut impl Rng) {
        match self {
            validator::ConsensusMsg::LeaderPrepare(msg) => msg.mutate(rng),
            validator::ConsensusMsg::LeaderCommit(msg) => msg.mutate(rng),
            validator::ConsensusMsg::ReplicaPrepare(msg) => msg.mutate(rng),
            validator::ConsensusMsg::ReplicaCommit(msg) => msg.mutate(rng),
        }
    }
}

impl Fuzz for validator::ReplicaPrepare {
    fn mutate(&mut self, rng: &mut impl Rng) {
        match rng.gen_range(0..4) {
            0 => self.view = rng.gen(),
            1 => self.high_vote.mutate(rng),
            2 => self.high_qc.mutate(rng),
            3 => self.protocol_version = rng.gen(),
            _ => unreachable!(),
        }
    }
}

impl Fuzz for validator::ReplicaCommit {
    fn mutate(&mut self, rng: &mut impl Rng) {
        match rng.gen_range(0..3) {
            0 => self.view = rng.gen(),
            1 => self.proposal.mutate(rng),
            2 => self.protocol_version = rng.gen(),
            _ => unreachable!(),
        }
    }
}

impl Fuzz for validator::LeaderPrepare {
    fn mutate(&mut self, rng: &mut impl Rng) {
        match rng.gen_range(0..3) {
            0 => self.proposal.mutate(rng),
            1 => self.justification.mutate(rng),
            2 => self.protocol_version = rng.gen(),
            _ => unreachable!(),
        }
    }
}

impl Fuzz for validator::LeaderCommit {
    fn mutate(&mut self, rng: &mut impl Rng) {
        match rng.gen_range(0..2) {
            0 => self.justification.mutate(rng),
            1 => self.protocol_version = rng.gen(),
            _ => unreachable!(),
        }
    }
}

impl Fuzz for validator::PrepareQC {
    fn mutate(&mut self, rng: &mut impl Rng) {
        // We give them different weights because we want to mutate the signature less often.
        match rng.gen_range(0..10) {
            0..=8 => {
                // Get keys from the aggregate QC map
                let keys = self.map.keys().cloned().collect::<Vec<_>>();

                // select random element from the vector
                let mut key = keys.choose(rng).unwrap().clone();

                // get value correspondent to the key
                let mut value = self.map.get(&key).unwrap().clone();

                // either mutate the key or the value.
                if rng.gen() {
                    self.map.remove(&key);
                    key.mutate(rng);
                } else {
                    value.mutate(rng);
                }

                self.map.insert(key, value);
            }
            9 => self.signature = rng.gen(),
            _ => unreachable!(),
        }
    }
}

impl Fuzz for validator::CommitQC {
    fn mutate(&mut self, rng: &mut impl Rng) {
        // We give them different weights because we want to mutate the message more often.
        match rng.gen_range(0..10) {
            0..=6 => self.message.mutate(rng),
            7 | 8 => self.signers.mutate(rng),
            9 => self.signature = rng.gen(),
            _ => unreachable!(),
        }
    }
}

impl Fuzz for validator::Signers {
    fn mutate(&mut self, rng: &mut impl Rng) {
        match rng.gen_range(0..5) {
            // Flips a random number of bits.
            0 => {
                for _ in 1..rng.gen_range(0..self.0.len()) {
                    self.0.set(rng.gen_range(0..self.0.len()), rng.gen());
                }
            }
            // Flips one random bit.
            1 => {
                let pos = rng.gen_range(0..self.0.len());
                let bit = self.0.get(pos).unwrap();
                self.0.set(pos, !bit);
            }
            // Sets all bits to false.
            2 => {
                self.0.set_all();
                self.0.negate();
            }
            // Sets all bits to true.
            3 => {
                self.0.set_all();
            }
            // Delete one bit.
            4 => {
                self.0.pop();
            }
            _ => unreachable!(),
        }
    }
}

impl Fuzz for validator::Payload {
    fn mutate(&mut self, rng: &mut impl Rng) {
        // Push bytes into the payload until it exceeds the limit.
        let num_bytes = MAX_PAYLOAD_SIZE - self.0.len() + 1;
        let bytes: Vec<u8> = (0..num_bytes).map(|_| rng.gen()).collect();
        self.0.extend_from_slice(&bytes);
        assert!(self.0.len() > MAX_PAYLOAD_SIZE);
    }
}

impl Fuzz for validator::BlockHeader {
    fn mutate(&mut self, rng: &mut impl Rng) {
        match rng.gen_range(0..3) {
            0 => self.parent = rng.gen(),
            1 => self.number = rng.gen(),
            2 => self.payload = rng.gen(),
            _ => unreachable!(),
        }
    }
}
