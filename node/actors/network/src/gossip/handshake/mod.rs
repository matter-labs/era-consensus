use super::Config;
use crate::{frame, noise};
use anyhow::Context as _;
use concurrency::{ctx, time};
use crypto::ByteFmt;
use roles::node;
use schema::proto::network::gossip as proto;
use zksync_protobuf::{read_required, required, ProtoFmt};

#[cfg(test)]
mod testonly;
#[cfg(test)]
mod tests;

/// Timeout on performing a handshake.
const TIMEOUT: time::Duration = time::Duration::seconds(5);

/// First message exchanged by nodes after establishing e2e encryption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Handshake {
    /// Session ID signed with the node key.
    /// Authenticates the peer to be the owner of the node key.
    pub(crate) session_id: node::Signed<node::SessionId>,
    /// Information whether the peer treats this connection as static.
    /// It is informational only, it doesn't affect the logic of the node.
    pub(crate) is_static: bool,
}

impl ProtoFmt for Handshake {
    type Proto = proto::Handshake;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            session_id: read_required(&r.session_id).context("session_id")?,
            is_static: *required(&r.is_static).context("is_static")?,
        })
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            session_id: Some(self.session_id.build()),
            is_static: Some(self.is_static),
        }
    }
}

/// Error returned by gossip handshake logic.
#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
    #[error("session id mismatch")]
    SessionIdMismatch,
    #[error("unexpected peer")]
    PeerMismatch,
    #[error("validator signature")]
    Signature(#[from] node::InvalidSignatureError),
    #[error("stream")]
    Stream(#[source] anyhow::Error),
}

pub(super) async fn outbound(
    ctx: &ctx::Ctx,
    cfg: &Config,
    stream: &mut noise::Stream,
    peer: &node::PublicKey,
) -> Result<(), Error> {
    let ctx = &ctx.with_timeout(TIMEOUT);
    let session_id = node::SessionId(stream.id().encode());
    frame::send_proto(
        ctx,
        stream,
        &Handshake {
            session_id: cfg.key.sign_msg(session_id.clone()),
            is_static: cfg.static_outbound.contains_key(peer),
        },
    )
    .await
    .map_err(Error::Stream)?;
    let h: Handshake = frame::recv_proto(ctx, stream)
        .await
        .map_err(Error::Stream)?;
    if h.session_id.msg != session_id {
        return Err(Error::SessionIdMismatch);
    }
    if &h.session_id.key != peer {
        return Err(Error::PeerMismatch);
    }
    h.session_id.verify()?;
    Ok(())
}

pub(super) async fn inbound(
    ctx: &ctx::Ctx,
    cfg: &Config,
    stream: &mut noise::Stream,
) -> Result<node::PublicKey, Error> {
    let ctx = &ctx.with_timeout(TIMEOUT);
    let session_id = node::SessionId(stream.id().encode());
    let h: Handshake = frame::recv_proto(ctx, stream)
        .await
        .map_err(Error::Stream)?;
    if h.session_id.msg != session_id {
        return Err(Error::SessionIdMismatch);
    }
    h.session_id.verify()?;
    frame::send_proto(
        ctx,
        stream,
        &Handshake {
            session_id: cfg.key.sign_msg(session_id.clone()),
            is_static: cfg.static_inbound.contains(&h.session_id.key),
        },
    )
    .await
    .map_err(Error::Stream)?;
    Ok(h.session_id.key)
}
