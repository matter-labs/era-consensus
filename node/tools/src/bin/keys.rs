//! This tool generates a validator/attester/node key pair and prints it to stdout.
#![allow(clippy::print_stdout)]

use crypto::TextFmt as _;
use zksync_consensus_crypto as crypto;
use zksync_consensus_roles::{attester, node, validator};

/// This tool generates a validator/attester/node key pair and prints it to stdout.
fn main() {
    let validator_key = validator::SecretKey::generate();
    let attester_key = attester::SecretKey::generate();
    let node_key = node::SecretKey::generate();
    println!("keys:");
    println!("{}", validator_key.encode());
    println!("{}", validator_key.public().encode());
    println!("{}", attester_key.encode());
    println!("{}", attester_key.public().encode());
    println!("{}", node_key.encode());
    println!("{}", node_key.public().encode());
}
