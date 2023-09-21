use super::node_config::{ConsensusConfig, GossipConfig, NodeConfig};
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
            validators: rng.gen(),
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

impl Distribution<NodeConfig> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> NodeConfig {
        NodeConfig {
            server_addr: make_addr(rng),
            metrics_server_addr: Some(make_addr(rng)),
            consensus: rng.gen(),
            gossip: rng.gen(),
            genesis_block: rng.gen(),
        }
    }
}

#[test]
fn test_schema_encoding() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    test_encode_random::<_, ConsensusConfig>(rng);
    test_encode_random::<_, GossipConfig>(rng);
    test_encode_random::<_, NodeConfig>(rng);
}
