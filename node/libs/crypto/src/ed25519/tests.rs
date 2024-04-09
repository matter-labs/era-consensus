use crate::ed25519::{PublicKey, SecretKey, Signature};
use crate::ByteFmt;

#[test]
fn test_ed25519() -> anyhow::Result<()> {
    struct TestVector {
        secret_key: Vec<u8>,
        public_key: Vec<u8>,
        message: Vec<u8>,
        signature: Vec<u8>,
    }

    // Test vectors obtained from https://github.com/dalek-cryptography/ed25519-dalek/blob/main/TESTVECTORS.
    let test_vectors: Vec<TestVector> = vec![
        TestVector {
            secret_key: hex::decode("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")?,
            public_key: hex::decode("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")?,
            message: hex::decode("")?,
            signature: hex::decode("e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b")?,
        },
        TestVector {
            secret_key: hex::decode("4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c")?,
            public_key: hex::decode("3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c")?,
            message: hex::decode("72")?,
            signature: hex::decode("92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c0072")?,
        },
        TestVector {
            secret_key: hex::decode("c5aa8df43f9f837bedb7442f31dcb7b166d38535076f094b85ce3a2e0b4458f7fc51cd8e6218a1a38da47ed00230f0580816ed13ba3303ac5deb911548908025")?,
            public_key: hex::decode("fc51cd8e6218a1a38da47ed00230f0580816ed13ba3303ac5deb911548908025")?,
            message: hex::decode("af82")?,
            signature: hex::decode("6291d657deec24024827e69c3abe01a30ce548a284743a445e3680d7db5ac3ac18ff9b538d16f290ae67f760984dc6594a7c15e9716ed28dc027beceea1ec40aaf82")?,
        },
    ];

    for test in &test_vectors {
        let sec_bytes = &test.secret_key[..32];
        let pub_bytes = &test.public_key[..32];
        let secret_key = SecretKey::decode(sec_bytes).unwrap();
        let public_key = PublicKey::decode(pub_bytes).unwrap();
        let msg_bytes = &test.message;

        assert_eq!(public_key, secret_key.public());

        let sig1: Signature = Signature::decode(&test.signature[..64]).unwrap();
        let sig2: Signature = secret_key.sign(msg_bytes);
        assert_eq!(
            sig1, sig2,
            "Signature bytes not equal on message {:?}",
            msg_bytes
        );
        assert!(
            public_key.verify(msg_bytes, &sig2).is_ok(),
            "Signature verification failed on message {:?}",
            msg_bytes
        );
    }
    Ok(())
}
