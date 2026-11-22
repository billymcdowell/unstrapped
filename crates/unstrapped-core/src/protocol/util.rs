//! Small little-endian helpers shared by the packet codecs.

use super::frame::FrameError;
use std::fmt;

/// Why a packet could not be decoded or encoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PacketError {
    /// The frame layer rejected the bytes.
    Frame(FrameError),
    /// A decoder was handed a frame of another packet type.
    WrongPacketType {
        /// Packet type the decoder handles.
        expected: u8,
        /// Packet type found.
        got: u8,
    },
    /// The payload ended before a required field.
    Truncated {
        /// Field or record being read.
        what: &'static str,
        /// Inner-record bytes needed.
        need: usize,
        /// Inner-record bytes present.
        have: usize,
    },
    /// The operation does not exist for this device family or layout (for example
    /// `GET_VERSION_INFO` on WHOOP 5, or encoding a record layout this crate cannot build).
    Unsupported(&'static str),
    /// A field holds a value that cannot be represented or is out of range.
    Invalid(&'static str),
}

impl fmt::Display for PacketError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PacketError::Frame(e) => write!(f, "frame error: {e}"),
            PacketError::WrongPacketType { expected, got } => {
                write!(f, "expected packet type {expected}, got {got}")
            }
            PacketError::Truncated { what, need, have } => {
                write!(f, "{what} truncated: need {need} bytes, have {have}")
            }
            PacketError::Unsupported(s) => write!(f, "unsupported: {s}"),
            PacketError::Invalid(s) => write!(f, "invalid value: {s}"),
        }
    }
}

impl std::error::Error for PacketError {}

impl From<FrameError> for PacketError {
    fn from(e: FrameError) -> Self {
        PacketError::Frame(e)
    }
}

pub(crate) fn need(what: &'static str, inner: &[u8], len: usize) -> Result<(), PacketError> {
    if inner.len() < len {
        Err(PacketError::Truncated { what, need: len, have: inner.len() })
    } else {
        Ok(())
    }
}

pub(crate) fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

pub(crate) fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

pub(crate) fn i16_at(b: &[u8], o: usize) -> i16 {
    i16::from_le_bytes([b[o], b[o + 1]])
}

pub(crate) fn f32_at(b: &[u8], o: usize) -> f32 {
    f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

pub(crate) fn put_u16(b: &mut [u8], o: usize, v: u16) {
    b[o..o + 2].copy_from_slice(&v.to_le_bytes());
}

pub(crate) fn put_u32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

pub(crate) fn put_i16(b: &mut [u8], o: usize, v: i16) {
    b[o..o + 2].copy_from_slice(&v.to_le_bytes());
}

pub(crate) fn put_f32(b: &mut [u8], o: usize, v: f32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

/// Unit of the R-R interval words on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RrUnit {
    /// The word is milliseconds. Always true on WHOOP 4. NOOP's shipping WHOOP 5 decoder also
    /// treats the word as milliseconds (see `docs/PROTOCOL_NOTES.md`, unknown U3).
    #[default]
    Milliseconds,
    /// The word is 1/1024 s ticks, converted with `(ticks * 1000 + 512) / 1024`. NOOP's written
    /// WHOOP 5 firmware-50.42.1.0 contract says this; its code does not. Opt in only if verified.
    Ticks1024,
}

impl RrUnit {
    /// Wire word to milliseconds.
    pub fn to_ms(self, raw: u16) -> u16 {
        match self {
            RrUnit::Milliseconds => raw,
            RrUnit::Ticks1024 => ((raw as u32 * 1000 + 512) / 1024).min(u16::MAX as u32) as u16,
        }
    }

    /// Milliseconds to wire word (inverse of [`to_ms`](Self::to_ms), rounded).
    pub fn from_ms(self, ms: u16) -> u16 {
        match self {
            RrUnit::Milliseconds => ms,
            RrUnit::Ticks1024 => ((ms as u32 * 1024 + 500) / 1000).min(u16::MAX as u32) as u16,
        }
    }
}

/// Knobs for family-specific decode/encode ambiguities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CodecOptions {
    /// Unit of WHOOP 5 R-R words in realtime (type 40) and historical (R18) records.
    pub whoop5_rr: RrUnit,
}

impl CodecOptions {
    pub(crate) fn rr_unit(&self, family: super::DeviceFamily) -> RrUnit {
        match family {
            super::DeviceFamily::Whoop4 => RrUnit::Milliseconds,
            super::DeviceFamily::Whoop5 => self.whoop5_rr,
        }
    }
}

/// Read up to `max` non-zero R-R words (`count` at `count_off`, words from `first_off`), bounded by
/// the record length. Zero words are placeholders and are omitted.
pub(crate) fn read_rr(
    inner: &[u8],
    count_off: usize,
    first_off: usize,
    max: usize,
    unit: RrUnit,
) -> Vec<u16> {
    let n = (inner[count_off] as usize).min(max);
    let mut out = Vec::new();
    for i in 0..n {
        let o = first_off + 2 * i;
        if o + 2 > inner.len() {
            break;
        }
        let v = u16_at(inner, o);
        if v != 0 {
            out.push(unit.to_ms(v));
        }
    }
    out
}

/// Write up to `max` R-R values and the count byte.
pub(crate) fn write_rr(
    inner: &mut [u8],
    count_off: usize,
    first_off: usize,
    max: usize,
    rr_ms: &[u16],
    unit: RrUnit,
) {
    let n = rr_ms.len().min(max);
    inner[count_off] = n as u8;
    for (i, &ms) in rr_ms.iter().take(n).enumerate() {
        put_u16(inner, first_off + 2 * i, unit.from_ms(ms));
    }
}
