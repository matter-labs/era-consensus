use std::{collections::BTreeMap, fmt};

use anyhow::Context as _;
use bit_vec::BitVec;
use zksync_consensus_crypto::{keccak256, ByteFmt, Text, TextFmt};
use zksync_consensus_utils::enum_util::{BadVariantError, Variant};

use crate::{attester, validator};

/// Message that is sent by an attester.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Msg {
    /// L1 batch message.
    Batch(attester::Batch),
}

impl Msg {
    /// Returns the hash of the message.
    ///
    /// TODO: Eventually this should be a hash over an ABI encoded payload that
    /// can be verified in Solidity.
    pub fn hash(&self) -> MsgHash {
        MsgHash(keccak256::Keccak256::new(&zksync_protobuf::canonical(self)))
    }
}

impl Variant<Msg> for attester::Batch {
    fn insert(self) -> Msg {
        Msg::Batch(self)
    }
    fn extract(msg: Msg) -> Result<Self, BadVariantError> {
        let Msg::Batch(this) = msg;
        Ok(this)
    }
}

/// Strongly typed signed l1 batch message.
/// WARNING: signature is not guaranteed to be valid.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Signed<V: Variant<Msg>> {
    /// The message that was signed.
    pub msg: V,
    /// The public key of the signer.
    pub key: attester::PublicKey,
    /// The signature.
    pub sig: attester::Signature,
}

/// Struct that represents a bit map of attesters. We use it to compactly store
/// which attesters signed a given Batch message.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Signers(pub BitVec);

impl Signers {
    /// Constructs an empty Signers set.
    pub fn new(n: usize) -> Self {
        Self(BitVec::from_elem(n, false))
    }

    /// Returns the number of signers, i.e. the number of attesters that signed
    /// the particular message that this signer bitmap refers to.
    pub fn count(&self) -> usize {
        self.0.iter().filter(|b| *b).count()
    }

    /// Size of the corresponding attester::Committee.
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Committee {
    vec: Vec<WeightedAttester>,
    indexes: BTreeMap<attester::PublicKey, usize>,
    total_weight: u64,
}

impl std::ops::Deref for Committee {
    type Target = Vec<WeightedAttester>;

    fn deref(&self) -> &Self::Target {
        &self.vec
    }
}

impl Committee {
    /// Creates a new Committee from a list of attester public keys.
    pub fn new(attesters: impl IntoIterator<Item = WeightedAttester>) -> anyhow::Result<Self> {
        let mut weighted_attester = BTreeMap::new();
        let mut total_weight: u64 = 0;
        for attester in attesters {
            anyhow::ensure!(
                !weighted_attester.contains_key(&attester.key),
                "Duplicated attester in attester Committee"
            );
            anyhow::ensure!(
                attester.weight > 0,
                "Attester weight has to be a positive value"
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

    /// Iterates over the attesters.
    pub fn iter(&self) -> impl Iterator<Item = &WeightedAttester> {
        self.vec.iter()
    }

    /// Iterates over attester keys.
    pub fn keys(&self) -> impl Iterator<Item = &attester::PublicKey> {
        self.vec.iter().map(|v| &v.key)
    }

    /// Returns the number of attesters.
    #[allow(clippy::len_without_is_empty)] // a valid `Committee` is always non-empty by construction
    pub fn len(&self) -> usize {
        self.vec.len()
    }

    /// Returns true if the given attester is in the attester committee.
    pub fn contains(&self, attester: &attester::PublicKey) -> bool {
        self.indexes.contains_key(attester)
    }

    /// Get attester by its index in the committee.
    pub fn get(&self, index: usize) -> Option<&WeightedAttester> {
        self.vec.get(index)
    }

    /// Get the index of a attester in the committee.
    pub fn index(&self, attester: &attester::PublicKey) -> Option<usize> {
        self.indexes.get(attester).copied()
    }

    /// Signature weight threshold for this attester committee.
    pub fn threshold(&self) -> u64 {
        threshold(self.total_weight())
    }

    /// Compute the sum of weights of as a list of public keys.
    ///
    /// The method assumes that the keys are unique and does not de-duplicate.
    pub fn weight(&self, key: &attester::PublicKey) -> Option<u64> {
        self.index(key).map(|i| self.vec[i].weight)
    }

    /// Compute the sum of weights of as a list of public keys.
    ///
    /// The method assumes that the keys are unique and does not de-duplicate.
    pub fn weight_of_keys<'a>(&self, keys: impl Iterator<Item = &'a attester::PublicKey>) -> u64 {
        keys.filter_map(|key| self.weight(key)).sum()
    }

    /// Compute the sum of weights of signers given as a bit vector.
    pub fn weight_of_signers(&self, signers: &Signers) -> u64 {
        assert_eq!(self.vec.len(), signers.len());
        self.vec
            .iter()
            .enumerate()
            .filter(|(i, _)| signers.0[*i])
            .map(|(_, v)| v.weight)
            .sum()
    }

    /// Sum of all attesters weight in the committee
    pub fn total_weight(&self) -> u64 {
        self.total_weight
    }
}

/// Calculate the attester threshold, that is the minimum votes weight for any attesters action to be valid,
/// for a given committee total weight.
/// Technically we need just n > f+1, but for now we use a threshold consistent with the validator committee.
pub fn threshold(total_weight: u64) -> u64 {
    total_weight - validator::max_faulty_weight(total_weight)
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

impl<V: Variant<Msg> + Clone> Signed<V> {
    /// Verify the signature on the message.
    pub fn verify(&self) -> anyhow::Result<()> {
        self.sig.verify_msg(&self.msg.clone().insert(), &self.key)
    }

    /// Casts a signed message variant to sub/super variant.
    /// It is an equivalent of constructing/deconstructing enum values.
    pub fn cast(self) -> Result<Signed<V>, BadVariantError> {
        Ok(Signed {
            msg: V::extract(self.msg.insert())?,
            key: self.key,
            sig: self.sig,
        })
    }
}

/// Attester representation inside a Committee.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeightedAttester {
    /// Attester key
    pub key: attester::PublicKey,
    /// Attester weight inside the Committee.
    pub weight: Weight,
}

/// Voting weight.
pub type Weight = u64;
