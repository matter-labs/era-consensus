//! This tool generates a validator key pair and prints it to stdout.
#![allow(clippy::print_stdout)]

use zksync_consensus_roles::validator;
use zksync_consensus_crypto as crypto;

/// This tool generates a validator key pair and prints it to stdout.
fn main() {
    let key = validator::SecretKey::generate(&mut rand::rngs::OsRng);
    let encoded_pk = crypto::TextFmt::encode(&key.public());
    let encoded_sk = crypto::TextFmt::encode(&key);
    println!("Generating keypair:");
    println!("{encoded_pk}");
    println!("{encoded_sk}");
}
