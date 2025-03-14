use std::net;

use anyhow::Context as _;
use zksync_concurrency::{ctx, limiter, time};

use super::*;
use crate::testonly::{test_encode_all_formats, FmtConv};

mod proto;

#[derive(Debug, PartialEq, Eq)]
enum B {
    U(bool),
    V(Box<B>),
}

impl ProtoFmt for B {
    type Proto = proto::B;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        use proto::b::T;
        Ok(match required(&r.t)? {
            T::U(x) => Self::U(*x),
            T::V(x) => Self::V(Box::new(ProtoFmt::read(x.as_ref())?)),
        })
    }
    fn build(&self) -> Self::Proto {
        use proto::b::T;
        let t = match self {
            Self::U(x) => T::U(*x),
            Self::V(x) => T::V(Box::new(x.build())),
        };
        Self::Proto { t: Some(t) }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct A {
    x: Vec<u8>,
    y: u64,
    e: Vec<i32>,
    b: B,
}

impl ProtoFmt for A {
    type Proto = proto::A;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            x: required(&r.x).context("x")?.clone(),
            y: *required(&r.y).context("y")?,
            e: r.e.clone(),
            b: read_required(&r.b).context("b")?,
        })
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            x: Some(self.x.clone()),
            y: Some(self.y),
            e: self.e.clone(),
            b: Some(self.b.build()),
        }
    }
}

#[test]
fn test_encode_decode() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let v = A {
        x: vec![1, 2, 3],
        y: 76,
        e: vec![8, 1, 9, 4],
        b: B::U(true),
    };
    testonly::test_encode(rng, &v);

    // Decoding should fail if there are trailing bytes present.
    let mut bytes = encode(&v);
    bytes.extend([4, 4, 3]);
    assert!(decode::<A>(&bytes).is_err());
}

#[test]
fn test_timestamp() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    let unix = time::UNIX_EPOCH;
    let zero = time::Duration::ZERO;
    let nano = time::Duration::nanoseconds(1);
    let sec = time::Duration::seconds(1);
    for d in [zero, nano, sec, 12345678 * sec - 3 * nano] {
        testonly::test_encode(rng, &(unix + d));
        testonly::test_encode(rng, &(unix - d));
    }
}

#[test]
fn test_socket_addr() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();
    for addr in [
        "[::1]:432",
        "74.223.12.1:63222",
        "127.0.0.1:0",
        "0.0.0.0:33",
    ] {
        let addr: net::SocketAddr = addr.parse().unwrap();
        testonly::test_encode(rng, &addr);
    }
}

#[test]
fn test_encoding() {
    let ctx = ctx::test_root(&ctx::RealClock);
    let rng = &mut ctx.rng();

    test_encode_all_formats::<FmtConv<net::SocketAddr>>(rng);
    test_encode_all_formats::<FmtConv<time::Duration>>(rng);
    test_encode_all_formats::<FmtConv<limiter::Rate>>(rng);
}
