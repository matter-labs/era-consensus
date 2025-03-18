//! Every connection starts with the following preface protocol:
//! 1. client sends Encryption msg to server.
//! 2. connection is upgraded to encrypted connection, according to the
//!    algorithm specified in the Encryption message.
//! 3. client sends Endpoint msg to server.
//! 4. client and server start endpoint-specific communication.
//!
//! Hence, the preface protocol is used to enable encryption
//! and multiplex between multiple endpoints available on the same TCP port.
use zksync_concurrency::{ctx, error::Wrap as _, time};
use zksync_protobuf::{kB, required, ProtoFmt};

use crate::{frame, metrics, noise, proto::preface as proto};

/// Timeout on executing the preface protocol.
const TIMEOUT: time::Duration = time::Duration::seconds(5);

/// Max size of the frames exchanged during preface.
const MAX_FRAME: usize = 10 * kB;

/// E2E encryption protocol to use on a TCP connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Encryption {
    /// Noise protocol in NN variant (see `noise` module).
    NoiseNN,
}

/// Endpoint that the client is trying to connect to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Endpoint {
    /// Consensus network endpoint.
    ConsensusNet,
    /// Gossip network endpoint.
    GossipNet,
}

impl ProtoFmt for Encryption {
    type Proto = proto::Encryption;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        use proto::encryption::T;
        Ok(match required(&r.t)? {
            T::NoiseNn(..) => Self::NoiseNN,
        })
    }
    fn build(&self) -> Self::Proto {
        use proto::encryption::T;
        let t = match self {
            Self::NoiseNN => T::NoiseNn(proto::encryption::NoiseNn {}),
        };
        Self::Proto { t: Some(t) }
    }
}

impl ProtoFmt for Endpoint {
    type Proto = proto::Endpoint;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        use proto::endpoint::T;
        Ok(match required(&r.t)? {
            T::ConsensusNet(..) => Self::ConsensusNet,
            T::GossipNet(..) => Self::GossipNet,
        })
    }
    fn build(&self) -> Self::Proto {
        use proto::endpoint::T;
        let t = match self {
            Self::ConsensusNet => T::ConsensusNet(proto::endpoint::ConsensusNet {}),
            Self::GossipNet => T::GossipNet(proto::endpoint::GossipNet {}),
        };
        Self::Proto { t: Some(t) }
    }
}

/// Connects to the given TCP address and performs client-side preface protocol.
pub(crate) async fn connect(
    ctx: &ctx::Ctx,
    addr: std::net::SocketAddr,
    endpoint: Endpoint,
) -> ctx::Result<noise::Stream> {
    let ctx = &ctx.with_timeout(TIMEOUT);
    let mut stream = metrics::MeteredStream::connect(ctx, addr)
        .await
        .wrap("connect()")?;
    frame::send_proto(ctx, &mut stream, &Encryption::NoiseNN)
        .await
        .wrap("frame::send_proto(encryption)")?;
    let mut stream = noise::Stream::client_handshake(ctx, stream)
        .await
        .wrap("client_handshake()")?;
    frame::send_proto(ctx, &mut stream, &endpoint)
        .await
        .wrap("frame::send_proto(endpoint)")?;
    Ok(stream)
}

/// Performs a server-side preface protocol.
pub(crate) async fn accept(
    ctx: &ctx::Ctx,
    mut stream: metrics::MeteredStream,
) -> ctx::Result<(noise::Stream, Endpoint)> {
    let ctx = &ctx.with_timeout(TIMEOUT);
    let encryption: Encryption = frame::recv_proto(ctx, &mut stream, MAX_FRAME)
        .await
        .wrap("recv_proto(encryption)")?;
    if encryption != Encryption::NoiseNN {
        return Err(anyhow::format_err!("unsupported encryption protocol: {encryption:?}").into());
    }
    let mut stream = noise::Stream::server_handshake(ctx, stream)
        .await
        .wrap("server_handshake()")?;
    let endpoint = frame::recv_proto(ctx, &mut stream, MAX_FRAME)
        .await
        .wrap("recv_proto(endpoint)")?;
    Ok((stream, endpoint))
}
