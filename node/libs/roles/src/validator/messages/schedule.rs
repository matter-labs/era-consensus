use std::collections::BTreeMap;

use anyhow::Context as _;
use num_bigint::BigUint;
use zksync_consensus_crypto::keccak256::Keccak256;
use zksync_protobuf::{read_required, required, ProtoFmt};

use super::ViewNumber;
use crate::{proto::validator as proto, validator};

/// A struct that represents a set of validators. It is used to store the current validator set.
/// We represent each validator by its validator public key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Schedule {
    vec: Vec<ValidatorInfo>,
    indexes: BTreeMap<validator::PublicKey, usize>,
    total_weight: u64,
    leaders: Vec<usize>,
    leader_selection: LeaderSelection,
    leader_weight: u64,
}

impl Schedule {
    /// Creates a new Schedule from a list of `ValidatorInfo` and a `LeaderSelection` mode.
    /// Note that the order of the given validators is NOT preserved in the schedule.
    pub fn new(
        validators: impl IntoIterator<Item = ValidatorInfo>,
        leader_selection: LeaderSelection,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !matches!(leader_selection.mode, LeaderSelectionMode::Sticky(_)),
            "Sticky leader selection mode is not supported"
        );

        let mut map = BTreeMap::new();
        let mut total_weight: u64 = 0;
        let mut leader_weight: u64 = 0;

        for v in validators {
            anyhow::ensure!(
                !map.contains_key(&v.key),
                "Duplicate key in validator Schedule"
            );
            anyhow::ensure!(v.weight > 0, "Validator weight has to be a positive value");

            total_weight = total_weight
                .checked_add(v.weight)
                .context("Sum of weights overflows in validator Schedule")?;
            if v.leader {
                // It can't overflow because we already checked that the total weight doesn't overflow.
                leader_weight += v.weight;
            }

            map.insert(v.key.clone(), v);
        }

        anyhow::ensure!(
            !map.is_empty(),
            "Validator Schedule must contain at least one validator"
        );

        let vec: Vec<_> = map.into_values().collect();

        let indexes: BTreeMap<validator::PublicKey, usize> = vec
            .iter()
            .enumerate()
            .map(|(i, v)| (v.key.clone(), i))
            .collect();

        let leaders: Vec<usize> = vec
            .iter()
            .enumerate()
            .filter_map(|(i, v)| if v.leader { Some(i) } else { None })
            .collect();

        anyhow::ensure!(
            !leaders.is_empty(),
            "Validator Schedule must contain at least one leader"
        );

        Ok(Self {
            vec,
            indexes,
            total_weight,
            leaders,
            leader_selection,
            leader_weight,
        })
    }

    /// Iterates over validators.
    pub fn iter(&self) -> impl Iterator<Item = &ValidatorInfo> {
        self.vec.iter()
    }

    /// Iterates over validator keys.
    pub fn keys(&self) -> impl Iterator<Item = &validator::PublicKey> {
        self.vec.iter().map(|v| &v.key)
    }

    /// Returns indexes of validators that are eligible to be leaders
    pub fn leaders(&self) -> &[usize] {
        &self.leaders
    }

    /// Returns the leader selection parameters.
    pub fn leader_selection(&self) -> &LeaderSelection {
        &self.leader_selection
    }

    /// Returns the number of validators.
    #[allow(clippy::len_without_is_empty)] // a valid `Schedule` is always non-empty by construction
    pub fn len(&self) -> usize {
        self.vec.len()
    }

    /// Returns true if the given validator is in the validator schedule.
    pub fn contains(&self, validator: &validator::PublicKey) -> bool {
        self.indexes.contains_key(validator)
    }

    /// Get validator by its index in the schedule.
    pub fn get(&self, index: usize) -> Option<&ValidatorInfo> {
        self.vec.get(index)
    }

    /// Get the index of a validator in the schedule.
    pub fn index(&self, validator: &validator::PublicKey) -> Option<usize> {
        self.indexes.get(validator).copied()
    }

    /// Sum of all validators' weight in the schedule
    pub fn total_weight(&self) -> u64 {
        self.total_weight
    }

    /// Signature weight threshold for this validator schedule.
    pub fn quorum_threshold(&self) -> u64 {
        quorum_threshold(self.total_weight())
    }

    /// Signature weight threshold for this validator schedule to trigger a reproposal.
    pub fn subquorum_threshold(&self) -> u64 {
        subquorum_threshold(self.total_weight())
    }

    /// Maximal weight of faulty replicas allowed in this validator schedule.
    pub fn max_faulty_weight(&self) -> u64 {
        max_faulty_weight(self.total_weight())
    }

    /// Computes the leader for the given view.
    pub fn view_leader(&self, view_number: ViewNumber) -> validator::PublicKey {
        let turn = view_number.0 / self.leader_selection.frequency;

        match &self.leader_selection.mode {
            LeaderSelectionMode::RoundRobin => {
                let index = self.leaders[turn as usize % self.leaders.len()];
                self.get(index).unwrap().key.clone()
            }
            LeaderSelectionMode::Weighted => {
                let eligibility =
                    LeaderSelection::leader_weighted_eligibility(turn, self.leader_weight);
                let mut offset = 0;
                for l in self.leaders.iter() {
                    let v = self.get(*l).unwrap();
                    offset += v.weight;
                    if eligibility < offset {
                        return v.key.clone();
                    }
                }
                unreachable!()
            }
            LeaderSelectionMode::Sticky(_) => unreachable!(),
        }
    }
}

impl ProtoFmt for Schedule {
    type Proto = proto::ValidatorSchedule;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        let validators: Vec<_> = r
            .validators
            .iter()
            .enumerate()
            .map(|(i, v)| ValidatorInfo::read(v).context(i))
            .collect::<Result<_, _>>()
            .context("validators")?;
        let leader_selection = read_required(&r.leader_selection).context("leader_selection")?;

        Self::new(validators, leader_selection)
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            validators: self.iter().map(|v| v.build()).collect(),
            leader_selection: Some(self.leader_selection.build()),
        }
    }
}

/// Validator representation inside a Schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorInfo {
    /// Validator key
    pub key: validator::PublicKey,
    /// Validator weight.
    pub weight: u64,
    /// Flag indicating if the validator is eligible to be a leader.
    pub leader: bool,
}

impl ProtoFmt for ValidatorInfo {
    type Proto = proto::ValidatorInfo;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            key: read_required(&r.key).context("key")?,
            weight: *required(&r.weight).context("weight")?,
            leader: *required(&r.leader).context("leader")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            key: Some(self.key.build()),
            weight: Some(self.weight),
            leader: Some(self.leader),
        }
    }
}

/// Leader selection parameters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeaderSelection {
    /// The number of views between leader changes. If it is 0 then the leader never rotates.
    pub frequency: u64,
    /// The mode used for selecting a new leader when we need to rotate.
    pub mode: LeaderSelectionMode,
}

impl LeaderSelection {
    /// Calculates the pseudo-random eligibility of a leader based on the input and total weight.
    fn leader_weighted_eligibility(input: u64, total_weight: u64) -> u64 {
        let input_bytes = input.to_be_bytes();
        let hash = Keccak256::new(&input_bytes);
        let hash_big = BigUint::from_bytes_be(hash.as_bytes());
        let total_weight_big = BigUint::from(total_weight);
        let ret_big = hash_big % total_weight_big;
        // Assumes that `ret_big` does not exceed 64 bits due to the modulo operation with a 64 bits-capped value.
        ret_big.to_u64_digits()[0]
    }
}

impl ProtoFmt for LeaderSelection {
    type Proto = proto::LeaderSelection;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            frequency: *required(&r.frequency).context("frequency")?,
            mode: read_required(&r.mode).context("mode")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            frequency: Some(self.frequency),
            mode: Some(self.mode.build()),
        }
    }
}

/// The mode used for selecting leader for a given view.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LeaderSelectionMode {
    /// Select in a round-robin fashion, based on validators' index within the set.
    RoundRobin,
    /// Select based on a sticky assignment to a specific validator.
    /// To be deprecated together with v1.
    Sticky(validator::PublicKey),
    /// Select pseudo-randomly, based on validators' weights.
    Weighted,
}

impl ProtoFmt for LeaderSelectionMode {
    type Proto = proto::LeaderSelectionMode;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        match required(&r.mode)? {
            proto::leader_selection_mode::Mode::RoundRobin(_) => {
                Ok(LeaderSelectionMode::RoundRobin)
            }
            proto::leader_selection_mode::Mode::Sticky(inner) => {
                let key = required(&inner.key).context("key")?;
                Ok(LeaderSelectionMode::Sticky(validator::PublicKey::read(
                    key,
                )?))
            }
            proto::leader_selection_mode::Mode::Weighted(_) => Ok(LeaderSelectionMode::Weighted),
        }
    }
    fn build(&self) -> Self::Proto {
        match self {
            LeaderSelectionMode::RoundRobin => proto::LeaderSelectionMode {
                mode: Some(proto::leader_selection_mode::Mode::RoundRobin(
                    proto::leader_selection_mode::RoundRobin {},
                )),
            },
            LeaderSelectionMode::Sticky(pk) => proto::LeaderSelectionMode {
                mode: Some(proto::leader_selection_mode::Mode::Sticky(
                    proto::leader_selection_mode::Sticky {
                        key: Some(pk.build()),
                    },
                )),
            },
            LeaderSelectionMode::Weighted => proto::LeaderSelectionMode {
                mode: Some(proto::leader_selection_mode::Mode::Weighted(
                    proto::leader_selection_mode::Weighted {},
                )),
            },
        }
    }
}

/// Calculate the maximum allowed weight for faulty replicas, for a given total weight.
pub fn max_faulty_weight(total_weight: u64) -> u64 {
    // Calculate the allowed maximum weight of faulty replicas. We want the following relationship to hold:
    //      n = 5*f + 1
    // for n total weight and f faulty weight. This results in the following formula for the maximum
    // weight of faulty replicas:
    //      f = floor((n - 1) / 5)
    (total_weight - 1) / 5
}

/// Calculate the consensus quorum threshold, the minimum votes' weight necessary to finalize a block,
/// for a given committee total weight.
pub fn quorum_threshold(total_weight: u64) -> u64 {
    total_weight - max_faulty_weight(total_weight)
}

/// Calculate the consensus subquorum threshold, the minimum votes' weight necessary to trigger a reproposal,
/// for a given committee total weight.
pub fn subquorum_threshold(total_weight: u64) -> u64 {
    total_weight - 3 * max_faulty_weight(total_weight)
}
