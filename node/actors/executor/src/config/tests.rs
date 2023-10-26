use super::{ConsensusConfig, ExecutorConfig, GossipConfig};
use concurrency::ctx;
use rand::{
    distributions::{Distribution, Standard},
    Rng,
};
use roles::{node, validator};
use schema::testonly::test_encode_random;

fn make_addr<R: Rng + ?Sized>(rng: &mut R) -> std::net::SocketAddr {
    std::net::SocketAddr::new(std::net::IpAddr::from(rng.gen::<[u8; 16]>()), rng.gen())
}

impl Distribution<ConsensusConfig> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ConsensusConfig {
        ConsensusConfig {
            key: rng.gen::<validator::SecretKey>().public(),
            public_addr: make_addr(rng),
        }
    }
}

impl Distribution<GossipConfig> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> GossipConfig {
        GossipConfig {
            key: rng.gen::<node::SecretKey>().public(),
            dynamic_inbound_limit: rng.gen(),
            static_inbound: (0..5)
                .map(|_| rng.gen::<node::SecretKey>().public())
                .collect(),
            static_outbound: (0..6)
                .map(|_| (rng.gen::<node::SecretKey>().public(), make_addr(rng)))
                .collect(),
        }
    }
}

impl Distribution<ExecutorConfig> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ExecutorConfig {
        ExecutorConfig {
            server_addr: make_addr(rng),
            gossip: rng.gen(),
            genesis_block: rng.gen(),
            validators: rng.gen(),
        }
    }
}

#[test]
fn test_schema_encoding() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    test_encode_random::<_, ConsensusConfig>(rng);
    test_encode_random::<_, GossipConfig>(rng);
    test_encode_random::<_, ExecutorConfig>(rng);
}
