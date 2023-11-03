#[test]
fn test_keccak256() -> Result<(), Box<dyn std::error::Error>> {
    use crate::keccak256::Keccak256;

    // Test vectors obtained from a trusted source
    let test_vectors: Vec<(&[u8], [u8; 32])> = vec![
        (
            b"testing",
            hex::decode("5f16f4c7f149ac4f9510d9cf8cf384038ad348b3bcdc01915f95de12df9d1b02")?
                .try_into()
                .unwrap(),
        ),
        (
            b"",
            hex::decode("c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470")?
                .try_into()
                .unwrap(),
        ),
        (
            &[0x12, 0x34, 0x56],
            hex::decode("6adf031833174bbe4c85eafe59ddb54e6584648c2c962c6f94791ab49caa0ad4")?
                .try_into()
                .unwrap(),
        ),
    ];

    for (input, expected_hash) in &test_vectors {
        let hash = Keccak256::new(input);
        assert_eq!(hash.as_bytes(), expected_hash);
    }

    Ok(())
}
