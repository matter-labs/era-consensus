use super::Connection;
use crate::{frame, noise, proto::gossip as proto, Config};
use anyhow::Context as _;
use std::sync::Arc;
use zksync_concurrency::{ctx, error::Wrap as _, time};
use zksync_consensus_crypto::ByteFmt;
use zksync_consensus_roles::{node, validator};
use zksync_protobuf::{kB, read_required, required, ProtoFmt};

#[cfg(test)]
mod testonly;
#[cfg(test)]
mod tests;

/// Timeout on performing a handshake.
const TIMEOUT: time::Duration = time::Duration::seconds(5);

/// Max size of a handshake frame.
const MAX_FRAME: usize = 10 * kB;

/// First message exchanged by nodes after establishing e2e encryption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Handshake {
    /// Session ID signed with the node key.
    /// Authenticates the peer to be the owner of the node key.
    pub(crate) session_id: node::Signed<node::SessionId>,
    /// Hash of the blockchain genesis specification.
    /// Only nodes with the same genesis belong to the same network.
    pub(crate) genesis: validator::GenesisHash,
    /// Information whether the peer treats this connection as static.
    /// It is informational only, it doesn't affect the logic of the node.
    pub(crate) is_static: bool,
    /// Version at which peer's binary has been built.
    /// It is declared by peer (i.e. not verified in any way).
    pub(crate) build_version: Option<semver::Version>,
}

impl ProtoFmt for Handshake {
    type Proto = proto::Handshake;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            session_id: read_required(&r.session_id).context("session_id")?,
            genesis: read_required(&r.genesis).context("genesis")?,
            is_static: *required(&r.is_static).context("is_static")?,
            build_version: r
                .build_version
                .as_ref()
                .map(|x| x.parse())
                .transpose()
                .context("build_version")?,
        })
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            session_id: Some(self.session_id.build()),
            genesis: Some(self.genesis.build()),
            is_static: Some(self.is_static),
            build_version: self.build_version.as_ref().map(|x| x.to_string()),
        }
    }
}

/// Error returned by gossip handshake logic.
#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
    #[error("genesis mismatch")]
    GenesisMismatch,
    #[error("session id mismatch")]
    SessionIdMismatch,
    #[error("unexpected peer")]
    PeerMismatch,
    #[error(transparent)]
    Signature(#[from] node::InvalidSignatureError),
    #[error(transparent)]
    Stream(#[from] ctx::Error),
}

pub(super) async fn outbound(
    ctx: &ctx::Ctx,
    cfg: &Config,
    genesis: validator::GenesisHash,
    stream: &mut noise::Stream,
    peer: &node::PublicKey,
) -> Result<Connection, Error> {
    let ctx = &ctx.with_timeout(TIMEOUT);
    let session_id = node::SessionId(stream.id().encode());
    frame::send_proto(
        ctx,
        stream,
        &Handshake {
            session_id: cfg.gossip.key.sign_msg(session_id.clone()),
            genesis,
            is_static: cfg.gossip.static_outbound.contains_key(peer),
            build_version: cfg.build_version.clone(),
        },
    )
    .await
    .wrap("send_proto()")?;
    let h: Handshake = frame::recv_proto(ctx, stream, MAX_FRAME)
        .await
        .wrap("recv_proto()")?;
    if h.genesis != genesis {
        return Err(Error::GenesisMismatch);
    }
    if h.session_id.msg != session_id {
        return Err(Error::SessionIdMismatch);
    }
    if &h.session_id.key != peer {
        return Err(Error::PeerMismatch);
    }
    h.session_id.verify()?;
    Ok(Connection {
        key: h.session_id.key,
        build_version: h.build_version,
        stats: stream.stats(),
    })
}

pub(super) async fn inbound(
    ctx: &ctx::Ctx,
    cfg: &Config,
    genesis: validator::GenesisHash,
    stream: &mut noise::Stream,
) -> Result<Arc<Connection>, Error> {
    let ctx = &ctx.with_timeout(TIMEOUT);
    let session_id = node::SessionId(stream.id().encode());
    let h: Handshake = frame::recv_proto(ctx, stream, MAX_FRAME)
        .await
        .wrap("recv_proto()")?;
    if h.session_id.msg != session_id {
        return Err(Error::SessionIdMismatch);
    }
    if h.genesis != genesis {
        return Err(Error::GenesisMismatch);
    }
    h.session_id.verify()?;
    frame::send_proto(
        ctx,
        stream,
        &Handshake {
            build_version: cfg.build_version.clone(),
            session_id: cfg.gossip.key.sign_msg(session_id.clone()),
            genesis,
            is_static: cfg.gossip.static_inbound.contains(&h.session_id.key),
        },
    )
    .await
    .wrap("send_proto()")?;
    Ok(Connection {
        key: h.session_id.key,
        build_version: h.build_version,
        stats: stream.stats(),
    }
    .into())
}
