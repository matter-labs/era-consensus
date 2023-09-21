//! Proto conversion for messages in std package.
use crate::{proto::std as proto, required, ProtoFmt};
use anyhow::Context as _;
use concurrency::time;
use std::net;

impl ProtoFmt for () {
    type Proto = proto::Void;
    fn read(_r: &Self::Proto) -> anyhow::Result<Self> {
        Ok(())
    }
    fn build(&self) -> Self::Proto {
        Self::Proto {}
    }
}

impl ProtoFmt for std::net::SocketAddr {
    type Proto = proto::SocketAddr;

    fn read(r: &Self::Proto) -> anyhow::Result<Self> {
        let ip = required(&r.ip).context("ip")?;
        let ip = match ip.len() {
            4 => net::IpAddr::from(<[u8; 4]>::try_from(&ip[..]).unwrap()),
            16 => net::IpAddr::from(<[u8; 16]>::try_from(&ip[..]).unwrap()),
            _ => anyhow::bail!("invalid ip length"),
        };
        let port = *required(&r.port).context("port")?;
        let port = u16::try_from(port).context("port")?;
        Ok(Self::new(ip, port))
    }

    fn build(&self) -> Self::Proto {
        Self::Proto {
            ip: Some(match self.ip() {
                net::IpAddr::V4(ip) => ip.octets().to_vec(),
                net::IpAddr::V6(ip) => ip.octets().to_vec(),
            }),
            port: Some(self.port() as u32),
        }
    }
}

impl ProtoFmt for time::Utc {
    type Proto = proto::Timestamp;

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
    type Proto = proto::Duration;

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
