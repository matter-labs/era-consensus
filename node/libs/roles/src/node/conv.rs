use crate::node;
use zksync_consensus_schema::{read_required, required, ProtoFmt};
use anyhow::Context as _;
use zksync_consensus_crypto::ByteFmt;
use zksync_consensus_utils::enum_util::Variant;

impl ProtoFmt for node::Msg {
    type Proto = node::schema::Msg;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        use node::schema::msg::T;
        Ok(match required(&r.t)? {
            T::SessionId(r) => Self::SessionId(node::SessionId(r.clone())),
        })
    }
    fn build(&self) -> Self::Proto {
        use node::schema::msg::T;
        let t = match self {
            Self::SessionId(x) => T::SessionId(x.0.clone()),
        };
        Self::Proto { t: Some(t) }
    }
}

impl ProtoFmt for node::PublicKey {
    type Proto = node::schema::PublicKey;
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
    type Proto = node::schema::Signature;
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
    type Proto = node::schema::Signed;
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
