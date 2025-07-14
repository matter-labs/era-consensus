#![allow(missing_docs)]

use std::iter::repeat_with;

use criterion::{criterion_group, criterion_main, Criterion};
use rand::Rng;

fn bench_bls12_381(c: &mut Criterion) {
    use zksync_consensus_crypto::bls12_381::{AggregateSignature, PublicKey, SecretKey, Signature};
    let mut rng = rand::thread_rng();
    let mut group = c.benchmark_group("bls12_381");
    group.bench_function("100 sig aggregation", |b| {
        b.iter(|| {
            let sks: Vec<SecretKey> = repeat_with(|| rng.gen::<SecretKey>()).take(100).collect();
            let pks: Vec<PublicKey> = sks.iter().map(|k| k.public()).collect();
            let msg = rng.gen::<[u8; 32]>();
            let sigs: Vec<Signature> = sks.iter().map(|k| k.sign(&msg)).collect();
            let agg = AggregateSignature::aggregate(&sigs);
            agg.verify(pks.iter().map(|pk| (&msg[..], pk)))
        });
    });

    group.finish();
}

criterion_group!(benches, bench_bls12_381);
criterion_main!(benches);
