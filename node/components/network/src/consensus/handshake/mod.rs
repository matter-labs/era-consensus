use anyhow::Context as _;
use zksync_concurrency::{ctx, error::Wrap as _, time};
use zksync_consensus_crypto::ByteFmt;
use zksync_consensus_roles::{node, validator};
use zksync_protobuf::{kB, read_required, ProtoFmt};

use crate::{frame, noise, proto::consensus as proto};

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
    /// Session ID signed with the validator key.
    /// Authenticates the peer to be the owner of the validator key.
    pub(crate) session_id: validator::Signed<node::SessionId>,
    /// Hash of the blockchain genesis specification.
    /// Only nodes with the same genesis belong to the same network.
    pub(crate) genesis: validator::GenesisHash,
}

impl ProtoFmt for Handshake {
    type Proto = proto::Handshake;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            session_id: read_required(&r.session_id).context("session_id")?,
            genesis: read_required(&r.genesis).context("genesis")?,
        })
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            session_id: Some(self.session_id.build()),
            genesis: Some(self.genesis.build()),
        }
    }
}

/// Error returned by handshake logic.
#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
    #[error("genesis mismatch")]
    GenesisMismatch,
    #[error("session id mismatch")]
    SessionIdMismatch,
    #[error("unexpected peer")]
    PeerMismatch,
    #[error("validator signature {0}")]
    Signature(#[from] anyhow::Error),
    #[error(transparent)]
    Stream(#[from] ctx::Error),
}

#[tracing::instrument(name = "handshake::outbound", skip_all)]
pub(super) async fn outbound(
    ctx: &ctx::Ctx,
    me: &validator::SecretKey,
    genesis: validator::GenesisHash,
    stream: &mut noise::Stream,
    peer: &validator::PublicKey,
) -> Result<(), Error> {
    let ctx = &ctx.with_timeout(TIMEOUT);
    let session_id = node::SessionId(stream.id().encode());
    frame::send_proto(
        ctx,
        stream,
        &Handshake {
            session_id: me.sign_msg(session_id.clone()),
            genesis,
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
    Ok(())
}

#[tracing::instrument(name = "handshake::inbound", skip_all)]
pub(super) async fn inbound(
    ctx: &ctx::Ctx,
    me: &validator::SecretKey,
    genesis: validator::GenesisHash,
    stream: &mut noise::Stream,
) -> Result<validator::PublicKey, Error> {
    let ctx = &ctx.with_timeout(TIMEOUT);
    let session_id = node::SessionId(stream.id().encode());
    let h: Handshake = frame::recv_proto(ctx, stream, MAX_FRAME)
        .await
        .wrap("recv_proto()")?;
    if h.genesis != genesis {
        return Err(Error::GenesisMismatch);
    }
    if h.session_id.msg != session_id.clone() {
        return Err(Error::SessionIdMismatch);
    }
    h.session_id.verify()?;
    frame::send_proto(
        ctx,
        stream,
        &Handshake {
            session_id: me.sign_msg(session_id.clone()),
            genesis,
        },
    )
    .await
    .wrap("send_proto()")?;
    Ok(h.session_id.key)
}
