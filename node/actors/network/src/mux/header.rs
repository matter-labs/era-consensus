#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct StreamId(pub(crate) u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct StreamKind(pub(super) u16);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct FrameKind(pub(super) u16);

impl FrameKind {
    pub(super) const MASK: u16 = Self::OPEN.0 | Self::DATA.0 | Self::CLOSE.0;
    pub(super) const OPEN: Self = Self(0b0000000000000000);
    pub(super) const DATA: Self = Self(0b0100000000000000);
    pub(super) const CLOSE: Self = Self(0b1000000000000000);
}

impl StreamKind {
    pub(super) const MASK: u16 = Self::ACCEPT.0 | Self::CONNECT.0;
    pub(super) const ACCEPT: Self = Self(0b0000000000000000);
    pub(super) const CONNECT: Self = Self(0b0010000000000000);
}

impl StreamId {
    pub(super) const MASK: u16 = 0b0001111111111111;

    pub(super) fn new(id: u16) -> StreamId {
        assert!(id <= Self::MASK);
        Self(id)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(super) struct Header(pub(crate) u16);

impl Header {
    pub(super) fn new(f: FrameKind, s: StreamKind, id: StreamId) -> Header {
        Header(f.0 | s.0 | id.0)
    }

    pub(super) fn frame_kind(self) -> FrameKind {
        FrameKind(self.0 & FrameKind::MASK)
    }

    pub(super) fn stream_kind(self) -> StreamKind {
        StreamKind(self.0 & StreamKind::MASK)
    }

    pub(super) fn stream_id(self) -> StreamId {
        StreamId(self.0 & StreamId::MASK)
    }

    pub(super) fn raw(self) -> [u8; 2] {
        self.0.to_le_bytes()
    }
}

impl From<[u8; 2]> for Header {
    fn from(raw: [u8; 2]) -> Self {
        Self(u16::from_le_bytes(raw))
    }
}
