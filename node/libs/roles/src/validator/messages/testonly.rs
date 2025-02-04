use super::*;
use crate::validator;
use crate::validator::messages::v1::{testonly::validator_committee, LeaderSelectionMode};
use zksync_consensus_crypto::Text;

/// Hardcoded payload.
pub fn payload() -> Payload {
    Payload(
        hex::decode("57b79660558f18d56b5196053f64007030a1cb7eeadb5c32d816b9439f77edf5f6bd9d")
            .unwrap(),
    )
}

/// Hardcoded validator secret keys.
pub fn validator_keys() -> Vec<validator::SecretKey> {
    [
        "validator:secret:bls12_381:27cb45b1670a1ae8d376a85821d51c7f91ebc6e32788027a84758441aaf0a987",
        "validator:secret:bls12_381:20132edc08a529e927f155e710ae7295a2a0d249f1b1f37726894d1d0d8f0d81",
        "validator:secret:bls12_381:0946901f0a6650284726763b12de5da0f06df0016c8ec2144cf6b1903f1979a6",
        "validator:secret:bls12_381:3143a64c079b2f50545288d7c9b282281e05c97ac043228830a9660ddd63fea3",
        "validator:secret:bls12_381:5512f40d33844c1c8107aa630af764005ab6e13f6bf8edb59b4ca3683727e619",
    ]
    .iter()
    .map(|raw| Text::new(raw).decode().unwrap())
    .collect()
}

/// Hardcoded genesis with no attesters.
pub fn genesis() -> Genesis {
    GenesisRaw {
        chain_id: ChainId(1337),
        fork_number: ForkNumber(42),
        first_block: BlockNumber(2834),

        protocol_version: ProtocolVersion(1),
        validators: validator_committee(),
        leader_selection: LeaderSelectionMode::Weighted,
    }
    .with_hash()
}
