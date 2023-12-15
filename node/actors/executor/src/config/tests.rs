use super::{ExecutorConfig};
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};
use zksync_concurrency::ctx;
use zksync_consensus_roles::{node, validator};
use zksync_protobuf::testonly::test_encode_random;

fn make_addr<R: Rng + ?Sized>(rng: &mut R) -> std::net::SocketAddr {
    std::net::SocketAddr::new(std::net::IpAddr::from(rng.gen::<[u8; 16]>()), rng.gen())
}

impl Distribution<ExecutorConfig> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ExecutorConfig {
        ExecutorConfig {
            server_addr: make_addr(rng),
            validators: rng.gen(),
            node_key: rng.gen().public(),
            
            gossip_dynamic_inbound_limit: rng.gen(),
            gossip_static_inbound: (0..5)
                .map(|_| rng.gen::<node::SecretKey>().public())
                .collect(),
            gossip_static_outbound: (0..6)
                .map(|_| (rng.gen::<node::SecretKey>().public(), make_addr(rng)))
                .collect(),
        }
    }
}

#[test]
fn test_schema_encoding() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    test_encode_random::<_, ExecutorConfig>(rng);
}
