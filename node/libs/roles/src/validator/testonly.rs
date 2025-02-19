//! Test-only utilities.
use super::{
    v1, Block, BlockNumber, ChainId, Committee, ForkNumber, Genesis, GenesisRaw, PreGenesisBlock,
    ProtocolVersion, SecretKey, WeightedValidator,
};
use crate::attester;
use rand::Rng;

/// Test setup specification.
#[derive(Debug, Clone)]
pub struct SetupSpec {
    /// ChainId
    pub chain_id: ChainId,
    /// Fork number.
    pub fork_number: ForkNumber,
    /// First block.
    pub first_block: BlockNumber,
    /// First block that exists.
    pub first_pregenesis_block: BlockNumber,
    /// Protocol version.
    pub protocol_version: ProtocolVersion,
    /// Validator secret keys and weights.
    pub validator_weights: Vec<(SecretKey, u64)>,
    /// Attester secret keys and weights.
    pub attester_weights: Vec<(attester::SecretKey, u64)>,
    /// Leader selection.
    pub leader_selection: v1::LeaderSelectionMode,
}

impl SetupSpec {
    /// New `SetupSpec`.
    pub fn new(rng: &mut impl Rng, validators: usize) -> Self {
        Self::new_with_weights(rng, vec![1; validators])
    }

    /// New `SetupSpec`.
    pub fn new_with_weights(rng: &mut impl Rng, weights: Vec<u64>) -> Self {
        let first_block = BlockNumber(rng.gen_range(0..100));
        Self {
            validator_weights: weights
                .clone()
                .into_iter()
                .map(|w| (rng.gen(), w))
                .collect(),
            attester_weights: weights.into_iter().map(|w| (rng.gen(), w)).collect(),
            chain_id: ChainId(1337),
            fork_number: ForkNumber(rng.gen_range(0..100)),
            first_block,
            first_pregenesis_block: BlockNumber(rng.gen_range(0..=first_block.0)),
            protocol_version: ProtocolVersion::CURRENT,
            leader_selection: v1::LeaderSelectionMode::RoundRobin,
        }
    }
}

/// Setup.
#[derive(Debug, Clone)]
pub struct SetupInner {
    /// Validators' secret keys.
    pub validator_keys: Vec<SecretKey>,
    /// Attesters' secret keys.
    pub attester_keys: Vec<attester::SecretKey>,
    /// Past blocks.
    pub blocks: Vec<Block>,
    /// Genesis config.
    pub genesis: Genesis,
}

impl std::ops::Deref for Setup {
    type Target = SetupInner;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Test setup.
#[derive(Debug, Clone)]
pub struct Setup(pub(crate) SetupInner);

impl Setup {
    /// New `Setup`.
    pub fn new(rng: &mut impl Rng, validators: usize) -> Self {
        let spec = SetupSpec::new(rng, validators);
        Self::from_spec(rng, spec)
    }

    /// New `Setup`.
    pub fn new_with_weights(rng: &mut impl Rng, weights: Vec<u64>) -> Self {
        let spec = SetupSpec::new_with_weights(rng, weights);
        Self::from_spec(rng, spec)
    }

    /// Generates a new `Setup` from the given `SetupSpec`.
    pub fn from_spec(rng: &mut impl Rng, spec: SetupSpec) -> Self {
        let mut this = Self(SetupInner {
            genesis: GenesisRaw {
                chain_id: spec.chain_id,
                fork_number: spec.fork_number,
                first_block: spec.first_block,

                protocol_version: spec.protocol_version,
                validators: Committee::new(spec.validator_weights.iter().map(|(k, w)| {
                    WeightedValidator {
                        key: k.public(),
                        weight: *w,
                    }
                }))
                .unwrap(),
                leader_selection: spec.leader_selection,
            }
            .with_hash(),
            validator_keys: spec.validator_weights.into_iter().map(|(k, _)| k).collect(),
            attester_keys: spec.attester_weights.into_iter().map(|(k, _)| k).collect(),
            blocks: vec![],
        });
        // Populate pregenesis blocks.
        for block in spec.first_pregenesis_block.0..spec.first_block.0 {
            this.0.blocks.push(
                PreGenesisBlock {
                    number: BlockNumber(block),
                    payload: rng.gen(),
                    justification: rng.gen(),
                }
                .into(),
            );
        }
        this
    }

    /// Next block to finalize.
    pub fn next(&self) -> BlockNumber {
        match self.0.blocks.last() {
            Some(b) => b.number().next(),
            None => self.0.genesis.first_block,
        }
    }

    /// Finds the block by the number.
    pub fn block(&self, n: BlockNumber) -> Option<&Block> {
        let first = self.0.blocks.first()?.number();
        self.0.blocks.get(n.0.checked_sub(first.0)? as usize)
    }
}
