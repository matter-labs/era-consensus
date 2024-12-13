//! This tool calculates a node/validator public key from its corresponding secret key and prints it to stdout.
#![allow(clippy::print_stdout)]

use crypto::TextFmt as _;
use std::io;
use zksync_consensus_crypto as crypto;
use zksync_consensus_roles::{node, validator};

/// This tool calculates a node/validator public key from its corresponding secret key and prints it to stdout.
fn main() {
    println!("Please enter the node secret key (don't trim the identifiers at the beginning) or leave empty to skip:");

    let mut input = String::new();

    io::stdin()
        .read_line(&mut input)
        .expect("Failed to read line");

    let input = input.trim();

    if !input.is_empty() {
        let text = crypto::Text::new(input);
        let secret_key = node::SecretKey::decode(text).expect("Failed to decode the secret key");
        println!("{}", secret_key.public().encode());
    }

    println!("Please enter the validator secret key (don't trim the identifiers at the beginning) or leave empty to skip:");

    let mut input = String::new();

    io::stdin()
        .read_line(&mut input)
        .expect("Failed to read line");

    let input = input.trim();

    if !input.is_empty() {
        let text = crypto::Text::new(input);
        let secret_key =
            validator::SecretKey::decode(text).expect("Failed to decode the secret key");
        println!("{}", secret_key.public().encode());
    }
}