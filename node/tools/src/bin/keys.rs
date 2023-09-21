//! This tool generates a validator key pair and prints it to stdout.
#![allow(clippy::print_stdout)]

use rand::Rng;
use roles::validator;

/// This tool generates a validator key pair and prints it to stdout.
fn main() {
    let key = validator::SecretKey::generate(rand::rngs::OsRng.gen());
    let encoded_pk = crypto::TextFmt::encode(&key.public());
    let encoded_sk = crypto::TextFmt::encode(&key);
    println!("Generating keypair:");
    println!("{encoded_pk}");
    println!("{encoded_sk}");
}
