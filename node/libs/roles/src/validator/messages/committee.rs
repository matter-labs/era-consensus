//! Messages related to the consensus protocol.
use std::collections::BTreeMap;

use anyhow::Context as _;
use zksync_protobuf::{read_required, required, ProtoFmt};

use crate::{proto::validator as proto, validator};

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

    /// Iterates over validators.
    pub fn iter(&self) -> impl Iterator<Item = &WeightedValidator> {
        self.vec.iter()
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

impl ProtoFmt for WeightedValidator {
    type Proto = proto::WeightedValidator;

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

/// Voting weight.
pub type Weight = u64;

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
