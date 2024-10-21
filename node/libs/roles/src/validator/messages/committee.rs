//! Messages related to the consensus protocol.
use super::{Signers, ViewNumber};
use crate::validator;
use anyhow::Context;
use num_bigint::BigUint;
use std::collections::BTreeMap;
use zksync_consensus_crypto::keccak256::Keccak256;

/// A struct that represents a set of validators. It is used to store the current validator set.
/// We represent each validator by its validator public key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Committee {
    vec: Vec<WeightedValidator>,
    indexes: BTreeMap<validator::PublicKey, usize>,
    total_weight: u64,
}

impl std::ops::Deref for Committee {
    type Target = Vec<WeightedValidator>;

    fn deref(&self) -> &Self::Target {
        &self.vec
    }
}

impl Committee {
    /// Creates a new Committee from a list of validator public keys. Note that the order of the given validators
    /// is NOT preserved in the committee.
    pub fn new(validators: impl IntoIterator<Item = WeightedValidator>) -> anyhow::Result<Self> {
        let mut map = BTreeMap::new();
        let mut total_weight: u64 = 0;
        for v in validators {
            anyhow::ensure!(
                !map.contains_key(&v.key),
                "Duplicate validator in validator Committee"
            );
            anyhow::ensure!(v.weight > 0, "Validator weight has to be a positive value");
            total_weight = total_weight
                .checked_add(v.weight)
                .context("Sum of weights overflows in validator Committee")?;
            map.insert(v.key.clone(), v);
        }
        anyhow::ensure!(
            !map.is_empty(),
            "Validator Committee must contain at least one validator"
        );
        let vec: Vec<_> = map.into_values().collect();
        Ok(Self {
            indexes: vec
                .iter()
                .enumerate()
                .map(|(i, v)| (v.key.clone(), i))
                .collect(),
            vec,
            total_weight,
        })
    }

    /// Iterates over validator keys.
    pub fn keys(&self) -> impl Iterator<Item = &validator::PublicKey> {
        self.vec.iter().map(|v| &v.key)
    }

    /// Returns the number of validators.
    #[allow(clippy::len_without_is_empty)] // a valid `Committee` is always non-empty by construction
    pub fn len(&self) -> usize {
        self.vec.len()
    }

    /// Returns true if the given validator is in the validator committee.
    pub fn contains(&self, validator: &validator::PublicKey) -> bool {
        self.indexes.contains_key(validator)
    }

    /// Get validator by its index in the committee.
    pub fn get(&self, index: usize) -> Option<&WeightedValidator> {
        self.vec.get(index)
    }

    /// Get the index of a validator in the committee.
    pub fn index(&self, validator: &validator::PublicKey) -> Option<usize> {
        self.indexes.get(validator).copied()
    }

    /// Computes the leader for the given view.
    pub fn view_leader(
        &self,
        view_number: ViewNumber,
        leader_selection: &LeaderSelectionMode,
    ) -> validator::PublicKey {
        match &leader_selection {
            LeaderSelectionMode::RoundRobin => {
                let index = view_number.0 as usize % self.len();
                self.get(index).unwrap().key.clone()
            }
            LeaderSelectionMode::Weighted => {
                let eligibility = LeaderSelectionMode::leader_weighted_eligibility(
                    view_number.0,
                    self.total_weight,
                );
                let mut offset = 0;
                for val in &self.vec {
                    offset += val.weight;
                    if eligibility < offset {
                        return val.key.clone();
                    }
                }
                unreachable!()
            }
            LeaderSelectionMode::Sticky(pk) => {
                let index = self.index(pk).unwrap();
                self.get(index).unwrap().key.clone()
            }
            LeaderSelectionMode::Rota(pks) => {
                let index = view_number.0 as usize % pks.len();
                let index = self.index(&pks[index]).unwrap();
                self.get(index).unwrap().key.clone()
            }
        }
    }

    /// Signature weight threshold for this validator committee.
    pub fn quorum_threshold(&self) -> u64 {
        quorum_threshold(self.total_weight())
    }

    /// Signature weight threshold for this validator committee to trigger a reproposal.
    pub fn subquorum_threshold(&self) -> u64 {
        subquorum_threshold(self.total_weight())
    }

    /// Maximal weight of faulty replicas allowed in this validator committee.
    pub fn max_faulty_weight(&self) -> u64 {
        max_faulty_weight(self.total_weight())
    }

    /// Compute the sum of signers weights.
    /// Panics if signers length does not match the number of validators in committee
    pub fn weight(&self, signers: &Signers) -> u64 {
        assert_eq!(self.vec.len(), signers.len());
        self.vec
            .iter()
            .enumerate()
            .filter(|(i, _)| signers.0[*i])
            .map(|(_, v)| v.weight)
            .sum()
    }

    /// Sum of all validators' weight in the committee
    pub fn total_weight(&self) -> u64 {
        self.total_weight
    }
}

/// Validator representation inside a Committee.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeightedValidator {
    /// Validator key
    pub key: validator::PublicKey,
    /// Validator weight inside the Committee.
    pub weight: Weight,
}

/// Voting weight;
pub type Weight = u64;

/// The mode used for selecting leader for a given view.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LeaderSelectionMode {
    /// Select in a round-robin fashion, based on validators' index within the set.
    RoundRobin,
    /// Select based on a sticky assignment to a specific validator.
    Sticky(validator::PublicKey),
    /// Select pseudo-randomly, based on validators' weights.
    Weighted,
    /// Select on a rotation of specific validator keys.
    Rota(Vec<validator::PublicKey>),
}

impl LeaderSelectionMode {
    /// Calculates the pseudo-random eligibility of a leader based on the input and total weight.
    pub fn leader_weighted_eligibility(input: u64, total_weight: u64) -> u64 {
        let input_bytes = input.to_be_bytes();
        let hash = Keccak256::new(&input_bytes);
        let hash_big = BigUint::from_bytes_be(hash.as_bytes());
        let total_weight_big = BigUint::from(total_weight);
        let ret_big = hash_big % total_weight_big;
        // Assumes that `ret_big` does not exceed 64 bits due to the modulo operation with a 64 bits-capped value.
        ret_big.to_u64_digits()[0]
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
