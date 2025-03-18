//! Proto conversion for messages in std package.
use anyhow::Context as _;
use zksync_concurrency::{limiter, time};

use crate::{proto, read_required, required, ProtoFmt};

impl ProtoFmt for () {
    type Proto = proto::std::Void;
    fn read(_r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(())
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {}
    }
}

impl ProtoFmt for std::net::SocketAddr {
    type Proto = proto::std::SocketAddr;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        let ip = required(&r.ip).context("ip")?;
        let ip = match ip.len() {
            4 => std::net::IpAddr::from(<[u8; 4]>::try_from(&ip[..]).unwrap()),
            16 => std::net::IpAddr::from(<[u8; 16]>::try_from(&ip[..]).unwrap()),
            _ => anyhow::bail!("invalid ip length"),
        };
        let port = *required(&r.port).context("port")?;
        let port = u16::try_from(port).context("port")?;
        Ok(Self::new(ip, port))
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            ip: Some(match self.ip() {
                std::net::IpAddr::V4(ip) => ip.octets().to_vec(),
                std::net::IpAddr::V6(ip) => ip.octets().to_vec(),
            }),
            port: Some(self.port() as u32),
        }
    }
}

impl ProtoFmt for time::Utc {
    type Proto = proto::std::Timestamp;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        let seconds = *required(&r.seconds).context("seconds")?;
        let nanos = *required(&r.nanos).context("nanos")?;
        Ok(time::UNIX_EPOCH + time::Duration::new(seconds, nanos))
    }

    fn build(&self) -> Self::Proto {
        let d = (*self - time::UNIX_EPOCH).build();
        Self::Proto {
            seconds: d.seconds,
            nanos: d.nanos,
        }
    }
}

impl ProtoFmt for time::Duration {
    type Proto = proto::std::Duration;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        let seconds = *required(&r.seconds).context("seconds")?;
        let nanos = *required(&r.nanos).context("nanos")?;
        Ok(Self::new(seconds, nanos))
    }

    fn build(&self) -> Self::Proto {
        let mut seconds = self.whole_seconds();
        let mut nanos = self.subsec_nanoseconds();
        if nanos < 0 {
            seconds -= 1;
            nanos += 1_000_000_000;
        }
        Self::Proto {
            seconds: Some(seconds),
            nanos: Some(nanos),
        }
    }
}

impl ProtoFmt for bit_vec::BitVec {
    type Proto = proto::std::BitVector;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        let size = *required(&r.size).context("size")? as usize;
        let mut this = Self::from_bytes(required(&r.bytes).context("bytes_")?);
        if this.len() < size {
            anyhow::bail!("'vector' has less than 'size' bits");
        }
        this.truncate(size);
        Ok(this)
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            size: Some(self.len() as u64),
            bytes: Some(self.to_bytes()),
        }
    }
}

impl ProtoFmt for limiter::Rate {
    type Proto = proto::std::RateLimit;
    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(Self {
            burst: required(&r.burst)
                .and_then(|x| Ok((*x).try_into()?))
                .context("burst")?,
            refresh: read_required(&r.refresh).context("refresh")?,
        })
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {
            burst: Some(self.burst.try_into().unwrap()),
            refresh: Some(self.refresh.build()),
        }
    }
}
