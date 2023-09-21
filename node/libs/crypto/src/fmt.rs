//! Traits for text (human readable) and byte encodings for crypto primitives.
use anyhow::Context as _;

/// Utility for parsing human-readable text representations via TextFmt::decode.
/// It keeps a reference to the initial text and a reference to the remaining unparsed text.
/// This allows to provide more context when a parsing error is encountered.
pub struct Text<'a> {
    /// Initial text.
    context: &'a str,
    /// Remaining unparsed text.
    inner: &'a str,
}

impl<'a> Text<'a> {
    /// Constructs a new unparsed text. Use other methods of Text
    /// to parse it afterwards. Text is an argument to TextFmt::decode
    /// trait method.
    pub fn new(s: &'a str) -> Self {
        Self {
            context: s,
            inner: s,
        }
    }

    /// Prefix of this text, which has been already parsed.
    fn prefix(&self) -> &'a str {
        &self.context[..self.context.len() - self.inner.len()]
        // ^ This should not panic since `self.inner` is a valid `str` by construction; thus,
        // the range end cannot break a multibyte UTF-8 character
    }

    /// Strips a fixed prefix from the remaining text.
    pub fn strip(mut self, prefix: &str) -> anyhow::Result<Self> {
        let Some(inner) = self.inner.strip_prefix(prefix) else {
            anyhow::bail!("{}: expected {} got {}", self.prefix(), prefix, self.inner);
        };
        self.inner = inner;
        Ok(self)
    }

    /// Parses the remaining text, assuming that it is in hex format.
    /// The parsed bytes are then converted to T, using ByteFmt trait.
    pub fn decode_hex<T: ByteFmt>(self) -> anyhow::Result<T> {
        let raw = hex::decode(self.inner).context(self.prefix().to_owned())?;
        ByteFmt::decode(&raw).context(self.prefix().to_owned())
    }

    /// Syntax sugar for `TextFmt::decode`:
    /// instead of `<T as TextFmt>::decode(t)`, you can write
    /// `t.decode::<T>()`.
    pub fn decode<T: TextFmt>(self) -> anyhow::Result<T> {
        TextFmt::decode(self)
    }
}

/// Trait converting a type from/to a human-readable text format.
/// It is roughly equivalent to str::FromStr + std::fmt::Display,
/// but has additional requirements:
/// - `x == decode(x.encode())` has to hold.
/// - encoding collision between different types should be unlikely.
///   For example, cryptographic keys of different types/roles should
///   not parse if type/role doesn't match.
/// - encoding should support backward-compatibility, at least in a
///   best effort manner (encoded strings will be included for example
///   in config files, so we want them to be decodable even after
///   upgrade of the binary).
pub trait TextFmt: Sized {
    /// Decodes the object from a text representation.
    fn decode(text: Text) -> anyhow::Result<Self>;
    /// Encodes the object to a text representation.
    fn encode(&self) -> String;
}

/// Decodes a required field from text representation.
pub fn read_required_text<T: TextFmt>(field: &Option<String>) -> anyhow::Result<T> {
    Text::new(field.as_ref().context("missing field")?).decode()
}

/// Decodes an optional field from text representation.
pub fn read_optional_text<T: TextFmt>(field: &Option<String>) -> anyhow::Result<Option<T>> {
    let Some(field) = field else { return Ok(None) };
    Text::new(field).decode().map(Some)
}

/// Trait converting a type from/to a sparse byte format.
/// It is roughly equivalent to serde::Serialize + serde::Deserialize,
/// but has additional requirements:
/// - binary encoding should be well defined, rather than rely on the internals
///   of the serde::Serializer implementation.
/// - encoding should support backward-compatibility. It will be used in building
///   network messages, which might get signed. Note that for signing it is nice to have
///   encoding uniqueness (i.e. `decode(b).encode()==b`). However it not strictly necessary,
///   because reencoding might be avoided.
pub trait ByteFmt: Sized {
    /// Decodes the object from the byte representation.
    fn decode(bytes: &[u8]) -> anyhow::Result<Self>;
    /// Encodes the object to the byte representation.
    fn encode(&self) -> Vec<u8>;
}

impl TextFmt for std::net::SocketAddr {
    fn decode(text: Text) -> anyhow::Result<Self> {
        Ok(text.inner.parse()?)
    }
    fn encode(&self) -> String {
        self.to_string()
    }
}
