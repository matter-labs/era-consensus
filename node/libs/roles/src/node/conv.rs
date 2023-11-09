use crate::{node, proto::node as proto};
use anyhow::Context as _;
use zksync_consensus_crypto::ByteFmt;
use zksync_consensus_utils::enum_util::Variant;
use zksync_protobuf::{read_required, required, ProtoFmt};

impl ProtoFmt for node::Msg {
    type Proto = proto::Msg;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        use proto::msg::T;
        Ok(match required(&r.t)? {
            T::SessionId(r) => Self::SessionId(node::SessionId(r.clone())),
        })
    }
    fn build(&self) -> Self::Proto {
        use proto::msg::T;
        let t = match self {
            Self::SessionId(x) => T::SessionId(x.0.clone()),
        };
        Self::Proto { t: Some(t) }
    }
}

impl ProtoFmt for node::PublicKey {
    type Proto = proto::PublicKey;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(ByteFmt::decode(required(&r.ed25519)?)?))
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            ed25519: Some(self.0.encode()),
        }
    }
}

impl ProtoFmt for node::Signature {
    type Proto = proto::Signature;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self(ByteFmt::decode(required(&r.ed25519)?)?))
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            ed25519: Some(self.0.encode()),
        }
    }
}

impl<V: Variant<node::Msg> + Clone> ProtoFmt for node::Signed<V> {
    type Proto = proto::Signed;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            msg: V::extract(read_required::<node::Msg>(&r.msg).context("msg")?)?,
            key: read_required(&r.key).context("key")?,
            sig: read_required(&r.sig).context("sig")?,
        })
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            msg: Some(self.msg.clone().insert().build()),
            key: Some(self.key.build()),
            sig: Some(self.sig.build()),
        }
    }
}
