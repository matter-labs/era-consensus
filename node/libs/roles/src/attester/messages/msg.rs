use std::{collections::BTreeMap, fmt};

use crate::{
    attester::{L1Batch, PublicKey, Signature},
    validator::ViewNumber,
};
use anyhow::Context;
use bit_vec::BitVec;
use zksync_consensus_crypto::{keccak256, ByteFmt, Text, TextFmt};
use zksync_consensus_utils::enum_util::{BadVariantError, Variant};

/// Message that is sent by an attester.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Msg {
    /// L1 batch message.
    L1Batch(L1Batch),
}

impl Msg {
    /// Returns the hash of the message.
    pub fn hash(&self) -> MsgHash {
        MsgHash(keccak256::Keccak256::new(&zksync_protobuf::canonical(self)))
    }
}

impl Variant<Msg> for L1Batch {
    fn insert(self) -> Msg {
        Msg::L1Batch(self)
    }
    fn extract(msg: Msg) -> Result<Self, BadVariantError> {
        let Msg::L1Batch(this) = msg;
        Ok(this)
    }
}

/// Strongly typed signed l1 batch message.
/// WARNING: signature is not guaranteed to be valid.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SignedBatchMsg<V: Variant<Msg>> {
    /// The message that was signed.
    pub msg: V,
    /// The public key of the signer.
    pub key: PublicKey,
    /// The signature.
    pub sig: Signature,
}

/// Struct that represents a bit map of validators. We use it to compactly store
/// which validators signed a given message.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Signers(pub BitVec);

impl Signers {
    /// Constructs an empty Signers set.
    pub fn new(n: usize) -> Self {
        Self(BitVec::from_elem(n, false))
    }

    /// Returns the number of signers, i.e. the number of validators that signed
    /// the particular message that this signer bitmap refers to.
    pub fn count(&self) -> usize {
        self.0.iter().filter(|b| *b).count()
    }

    /// Size of the corresponding ValidatorSet.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns true if there are no signers.
    pub fn is_empty(&self) -> bool {
        self.0.none()
    }
}

/// A struct that represents a set of attesters. It is used to store the current attester set.
/// We represent each attester by its attester public key.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Committee {
    vec: Vec<WeightedAttester>,
    indexes: BTreeMap<PublicKey, usize>,
    total_weight: u64,
}

impl Committee {
    /// Creates a new Committee from a list of validator public keys.
    pub fn new(attesters: impl IntoIterator<Item = WeightedAttester>) -> anyhow::Result<Self> {
        let mut weighted_attester = BTreeMap::new();
        let mut total_weight: u64 = 0;
        for attester in attesters {
            anyhow::ensure!(
                !weighted_attester.contains_key(&attester.key),
                "Duplicate validator in validator Committee"
            );
            anyhow::ensure!(
                attester.weight > 0,
                "Validator weight has to be a positive value"
            );
            total_weight = total_weight
                .checked_add(attester.weight)
                .context("Sum of weights overflows in attester Committee")?;
            weighted_attester.insert(attester.key.clone(), attester);
        }
        anyhow::ensure!(
            !weighted_attester.is_empty(),
            "Attester Committee must contain at least one attester"
        );
        println!("weighted_attesters: {:?}", total_weight);
        Ok(Self {
            vec: weighted_attester.values().cloned().collect(),
            indexes: weighted_attester
                .values()
                .enumerate()
                .map(|(i, v)| (v.key.clone(), i))
                .collect(),
            total_weight,
        })
    }

    /// Iterates over weighted validators.
    pub fn iter(&self) -> impl Iterator<Item = &WeightedAttester> {
        self.vec.iter()
    }

    /// Iterates over validator keys.
    pub fn iter_keys(&self) -> impl Iterator<Item = &PublicKey> {
        self.vec.iter().map(|v| &v.key)
    }

    /// Returns the number of validators.
    #[allow(clippy::len_without_is_empty)] // a valid `Committee` is always non-empty by construction
    pub fn len(&self) -> usize {
        self.vec.len()
    }

    /// Returns true if the given validator is in the validator committee.
    pub fn contains(&self, validator: &PublicKey) -> bool {
        self.indexes.contains_key(validator)
    }

    /// Get validator by its index in the committee.
    pub fn get(&self, index: usize) -> Option<&WeightedAttester> {
        self.vec.get(index)
    }

    /// Get the index of a validator in the committee.
    pub fn index(&self, validator: &PublicKey) -> Option<usize> {
        self.indexes.get(validator).copied()
    }

    /// Computes the leader for the given view.
    pub fn view_leader(&self, view_number: ViewNumber) -> PublicKey {
        let index = view_number.0 as usize % self.len();
        self.get(index).unwrap().key.clone()
    }

    /// Signature weight threshold for this validator committee.
    pub fn threshold(&self) -> u64 {
        threshold(self.total_weight())
    }

    /// Compute the sum of signers weights.
    /// Panics if signers length does not match the number of attesters in committee
    pub fn weight(&self, signers: &Signers) -> u64 {
        println!("attester.vec.len: {:?}", self.vec.len());
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

    /// Maximal weight of faulty replicas allowed in this validator committee.
    pub fn max_faulty_weight(&self) -> u64 {
        max_faulty_weight(self.total_weight())
    }
}

/// Calculate the maximum number of faulty replicas, for a given number of replicas.
pub fn faulty_replicas(n: usize) -> usize {
    // Calculate the allowed maximum number of faulty replicas. We want the following relationship to hold:
    //      n = 5*f + 1
    // for n total replicas and f faulty replicas. This results in the following formula for the maximum
    // number of faulty replicas:
    //      f = floor((n - 1) / 5)
    // Because of this, it doesn't make sense to have 5*f + 2 or 5*f + 3 replicas. It won't increase the number
    // of allowed faulty replicas.
    (n - 1) / 5
}

/// Calculate the consensus threshold, the minimum votes' weight for any consensus action to be valid,
/// for a given committee total weight.
pub fn threshold(total_weight: u64) -> u64 {
    total_weight - max_faulty_weight(total_weight)
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

impl std::ops::BitOrAssign<&Self> for Signers {
    fn bitor_assign(&mut self, other: &Self) {
        self.0.or(&other.0);
    }
}

impl std::ops::BitAndAssign<&Self> for Signers {
    fn bitand_assign(&mut self, other: &Self) {
        self.0.and(&other.0);
    }
}

impl std::ops::BitAnd for &Signers {
    type Output = Signers;
    fn bitand(self, other: Self) -> Signers {
        let mut this = self.clone();
        this &= other;
        this
    }
}

/// The hash of a message.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MsgHash(pub(crate) keccak256::Keccak256);

impl ByteFmt for MsgHash {
    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        ByteFmt::decode(bytes).map(Self)
    }

    fn encode(&self) -> Vec<u8> {
        ByteFmt::encode(&self.0)
    }
}

impl TextFmt for MsgHash {
    fn decode(text: Text) -> anyhow::Result<Self> {
        text.strip("attester_msg:keccak256:")?
            .decode_hex()
            .map(Self)
    }

    fn encode(&self) -> String {
        format!(
            "attester_msg:keccak256:{}",
            hex::encode(ByteFmt::encode(&self.0))
        )
    }
}

impl fmt::Debug for MsgHash {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.write_str(&TextFmt::encode(self))
    }
}

impl<V: Variant<Msg> + Clone> SignedBatchMsg<V> {
    /// Verify the signature on the message.
    pub fn verify(&self) -> anyhow::Result<()> {
        self.sig.verify_msg(&self.msg.clone().insert(), &self.key)
    }

    /// Casts a signed message variant to sub/super variant.
    /// It is an equivalent of constructing/deconstructing enum values.
    pub fn cast(self) -> Result<SignedBatchMsg<V>, BadVariantError> {
        Ok(SignedBatchMsg {
            msg: V::extract(self.msg.insert())?,
            key: self.key,
            sig: self.sig,
        })
    }
}

/// Attester representation inside a Committee.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeightedAttester {
    /// Validator key
    pub key: PublicKey,
    /// Validator weight inside the Committee.
    pub weight: u64,
}
