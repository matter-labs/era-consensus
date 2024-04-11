use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use crate::{
    attester::{L1Batch, PublicKey, Signature},
    validator::ViewNumber,
};
use bit_vec::BitVec;
use zksync_consensus_crypto::{keccak256, ByteFmt, Text, TextFmt};
use zksync_consensus_utils::enum_util::{BadVariantError, Variant};

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
        let Msg::L1Batch(this) = msg else {
            return Err(BadVariantError);
        };
        Ok(this)
    }
}

/// Strongly typed signed l1 batch message.
/// WARNING: signature is not guaranteed to be valid.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SignedBatchMsg {
    /// The message that was signed.
    pub msg: L1Batch,
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

/// A struct that represents a set of validators. It is used to store the current validator set.
/// We represent each validator by its validator public key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttesterSet {
    vec: Vec<PublicKey>,
    map: BTreeMap<PublicKey, usize>,
}

impl AttesterSet {
    /// Creates a new AttesterSet from a list of validator public keys.
    pub fn new(attesters: impl IntoIterator<Item = PublicKey>) -> anyhow::Result<Self> {
        let mut set = BTreeSet::new();
        for attester in attesters {
            anyhow::ensure!(set.insert(attester), "Duplicate validator in ValidatorSet");
        }
        anyhow::ensure!(
            !set.is_empty(),
            "ValidatorSet must contain at least one validator"
        );
        Ok(Self {
            vec: set.iter().cloned().collect(),
            map: set.into_iter().enumerate().map(|(i, pk)| (pk, i)).collect(),
        })
    }

    /// Iterates over validators.
    pub fn iter(&self) -> impl Iterator<Item = &PublicKey> {
        self.vec.iter()
    }

    /// Returns the number of validators.
    #[allow(clippy::len_without_is_empty)] // a valid `ValidatorSet` is always non-empty by construction
    pub fn len(&self) -> usize {
        self.vec.len()
    }

    /// Returns true if the given validator is in the validator set.
    pub fn contains(&self, validator: &PublicKey) -> bool {
        self.map.contains_key(validator)
    }

    /// Get validator by its index in the set.
    pub fn get(&self, index: usize) -> Option<&PublicKey> {
        self.vec.get(index)
    }

    /// Get the index of a validator in the set.
    pub fn index(&self, validator: &PublicKey) -> Option<usize> {
        self.map.get(validator).copied()
    }

    /// Computes the validator for the given view.
    pub fn view_leader(&self, view_number: ViewNumber) -> PublicKey {
        let index = view_number.0 as usize % self.len();
        self.get(index).unwrap().clone()
    }

    // /// Signature threshold for this validator set.
    // pub fn threshold(&self) -> usize {
    //     threshold(self.len())
    // }

    // /// Maximal number of faulty replicas allowed in this validator set.
    // pub fn faulty_replicas(&self) -> usize {
    //     faulty_replicas(self.len())
    // }
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

impl SignedBatchMsg {
    /// Verify the signature on the message.
    pub fn verify(&self) -> anyhow::Result<()> {
        self.sig.verify_msg(&self.msg.clone().insert(), &self.key)
    }

    /// Casts a signed message variant to sub/super variant.
    /// It is an equivalent of constructing/deconstructing enum values.
    pub fn cast(self) -> Result<SignedBatchMsg, BadVariantError> {
        Ok(SignedBatchMsg {
            msg: L1Batch::extract(self.msg.insert())?,
            key: self.key,
            sig: self.sig,
        })
    }
}
