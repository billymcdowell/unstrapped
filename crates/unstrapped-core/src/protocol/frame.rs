//! Frame envelope: build and validate complete frames for both families.
//!
//! ```text
//! WHOOP 4                                   WHOOP 5 / MG ("format 1")
//! [0]    0xAA                               [0]    0xAA
//! [1..3) length u16 LE = inner + 4          [1]    0x01 (format)
//! [3]    crc8(bytes 1..3)                   [2..4) declared length u16 LE = padded inner + 4
//! [4..)  inner = type, seq, cmd, payload    [4..6) header flags (host->strap 00 01, strap->host 01 00)
//! [len..len+4) crc32(inner) LE              [6..8) crc16-modbus(bytes 0..6) LE
//! total = length + 4                        [8..)  inner, zero padded to a multiple of 4
//!                                           [end-4..end) crc32(padded inner) LE
//!                                           total = declared length + 8
//! ```
//!
//! The *inner record* (`type, seq, cmd, payload`) is identical in shape for both families, which is
//! why [`Frame`] is family neutral. The meaning of `seq` and `cmd` depends on the packet type:
//! commands carry a sequence number and a command number; command responses carry the strap's own
//! sequence and the command being answered; historical records carry the record layout version in
//! `seq` and a marker byte in `cmd`.

use super::DeviceFamily;
use super::crc::{crc8, crc16_modbus, crc32};
use std::fmt;

/// Start-of-frame marker.
pub const SOF: u8 = 0xAA;

/// Largest frame the reassembler will wait for. Real frames reach about 2.2 KB (WHOOP 5 R20 is
/// 2152 bytes; WHOOP 4 raw is 1928); anything beyond this is a corrupt length, not a frame.
pub const MAX_FRAME_BYTES: usize = 8192;

/// WHOOP 5 format byte (`frame[1]`).
pub const W5_FORMAT_1: u8 = 0x01;

/// Packet type byte values (`inner[0]`). Names follow NOOP's schema.
pub mod packet_type {
    /// Host to strap command.
    pub const COMMAND: u8 = 35;
    /// Strap reply to a command.
    pub const COMMAND_RESPONSE: u8 = 36;
    /// Older WHOOP 5 label for a command; role in firmware 50.42.1.0 unconfirmed.
    pub const PUFFIN_COMMAND: u8 = 37;
    /// Older WHOOP 5 label for a command response; treated like [`COMMAND_RESPONSE`].
    pub const PUFFIN_COMMAND_RESPONSE: u8 = 38;
    /// Live heart rate and R-R.
    pub const REALTIME_DATA: u8 = 40;
    /// Live raw sensor data (IMU / optical / ECG). Not decoded by this crate.
    pub const REALTIME_RAW_DATA: u8 = 43;
    /// Historical record.
    pub const HISTORICAL_DATA: u8 = 47;
    /// Strap event.
    pub const EVENT: u8 = 48;
    /// History offload metadata (start / end / complete).
    pub const METADATA: u8 = 49;
    /// Firmware console log text.
    pub const CONSOLE_LOGS: u8 = 50;
    /// WHOOP 5 metadata alias, handled like [`METADATA`].
    pub const PUFFIN_METADATA: u8 = 56;
}

/// Why a frame was rejected, or could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// First byte is not `0xAA` (or the input is empty).
    NoStartOfFrame,
    /// Fewer bytes than the family's smallest legal frame.
    TooShort {
        /// Bytes supplied.
        have: usize,
        /// Minimum for the family.
        need: usize,
    },
    /// The length field declares a frame smaller than the family minimum.
    DeclaredTooSmall {
        /// Total frame size implied by the length field.
        declared_total: usize,
    },
    /// The length field declares more than [`MAX_FRAME_BYTES`].
    DeclaredTooLarge {
        /// Total frame size implied by the length field.
        declared_total: usize,
    },
    /// Supplied byte count differs from the declared total (truncated, or trailing bytes).
    LengthMismatch {
        /// Total frame size implied by the length field.
        declared_total: usize,
        /// Bytes supplied.
        actual: usize,
    },
    /// Header checksum (CRC-8 on WHOOP 4, CRC-16/MODBUS on WHOOP 5) disagrees.
    HeaderCrc {
        /// Value computed over the header.
        expected: u16,
        /// Value found in the frame.
        actual: u16,
    },
    /// WHOOP 5 format byte is not `0x01`.
    UnsupportedFormat(u8),
    /// CRC-32 over the inner record disagrees.
    PayloadCrc {
        /// Value computed over the inner record.
        expected: u32,
        /// Value found in the frame.
        actual: u32,
    },
    /// Encode only: inner record does not fit the 16-bit length field / [`MAX_FRAME_BYTES`].
    TooLong {
        /// Total frame size that would result.
        total: usize,
    },
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::NoStartOfFrame => write!(f, "missing 0xAA start-of-frame"),
            FrameError::TooShort { have, need } => write!(f, "frame too short: {have} < {need} bytes"),
            FrameError::DeclaredTooSmall { declared_total } => {
                write!(f, "declared frame size {declared_total} is below the family minimum")
            }
            FrameError::DeclaredTooLarge { declared_total } => {
                write!(f, "declared frame size {declared_total} exceeds {MAX_FRAME_BYTES}")
            }
            FrameError::LengthMismatch { declared_total, actual } => {
                write!(f, "declared frame size {declared_total} but {actual} bytes supplied")
            }
            FrameError::HeaderCrc { expected, actual } => {
                write!(f, "header checksum mismatch: computed {expected:#06x}, frame has {actual:#06x}")
            }
            FrameError::UnsupportedFormat(b) => write!(f, "unsupported WHOOP 5 frame format byte {b:#04x}"),
            FrameError::PayloadCrc { expected, actual } => {
                write!(f, "CRC32 mismatch: computed {expected:#010x}, frame has {actual:#010x}")
            }
            FrameError::TooLong { total } => write!(f, "frame of {total} bytes is too long to encode"),
        }
    }
}

impl std::error::Error for FrameError {}

/// A decoded (or to-be-encoded) frame: the inner record plus, on WHOOP 5, the two header flag bytes.
///
/// On WHOOP 5 the inner record is zero padded to a multiple of four bytes before it is sent, and
/// the padding is indistinguishable from payload, so a *decoded* WHOOP 5 `payload` may carry up to
/// three trailing zero bytes beyond what the sender supplied. Fixed-layout decoders ignore them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// `inner[0]`: packet type (see [`packet_type`]).
    pub packet_type: u8,
    /// `inner[1]`: sequence number, or record version for historical data.
    pub seq: u8,
    /// `inner[2]`: command number, event number, metadata type, or record marker.
    pub cmd: u8,
    /// `inner[3..]`.
    pub payload: Vec<u8>,
    /// WHOOP 5 header flag bytes (`frame[4..6]`). `None` on WHOOP 4 and when building a frame
    /// (the default is `00 01` for command packet types, `01 00` otherwise).
    pub header: Option<[u8; 2]>,
}

impl Frame {
    /// Build a frame with no explicit WHOOP 5 header bytes.
    pub fn new(packet_type: u8, seq: u8, cmd: u8, payload: impl Into<Vec<u8>>) -> Self {
        Frame { packet_type, seq, cmd, payload: payload.into(), header: None }
    }

    /// Set the WHOOP 5 header flag bytes explicitly.
    pub fn with_header(mut self, header: [u8; 2]) -> Self {
        self.header = Some(header);
        self
    }

    /// The inner record: `type, seq, cmd, payload`.
    pub fn inner(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(3 + self.payload.len());
        v.extend_from_slice(&[self.packet_type, self.seq, self.cmd]);
        v.extend_from_slice(&self.payload);
        v
    }

    fn default_w5_header(&self) -> [u8; 2] {
        match self.packet_type {
            packet_type::COMMAND | packet_type::PUFFIN_COMMAND => [0x00, 0x01],
            _ => [0x01, 0x00],
        }
    }

    /// Serialize to the on-wire bytes for `family`, computing every checksum and (WHOOP 5) padding.
    pub fn encode(&self, family: DeviceFamily) -> Result<Vec<u8>, FrameError> {
        let mut inner = self.inner();
        match family {
            DeviceFamily::Whoop4 => {
                let length = inner.len() + 4;
                let total = length + 4;
                if length > u16::MAX as usize || total > MAX_FRAME_BYTES {
                    return Err(FrameError::TooLong { total });
                }
                let len_bytes = (length as u16).to_le_bytes();
                let mut out = Vec::with_capacity(total);
                out.push(SOF);
                out.extend_from_slice(&len_bytes);
                out.push(crc8(&len_bytes));
                out.extend_from_slice(&inner);
                out.extend_from_slice(&crc32(&inner).to_le_bytes());
                Ok(out)
            }
            DeviceFamily::Whoop5 => {
                while !inner.len().is_multiple_of(4) {
                    inner.push(0);
                }
                let declared = inner.len() + 4;
                let total = declared + 8;
                if declared > u16::MAX as usize || total > MAX_FRAME_BYTES {
                    return Err(FrameError::TooLong { total });
                }
                let hdr = self.header.unwrap_or_else(|| self.default_w5_header());
                let dl = (declared as u16).to_le_bytes();
                let mut out = Vec::with_capacity(total);
                out.extend_from_slice(&[SOF, W5_FORMAT_1, dl[0], dl[1], hdr[0], hdr[1]]);
                let c16 = crc16_modbus(&out);
                out.extend_from_slice(&c16.to_le_bytes());
                out.extend_from_slice(&inner);
                out.extend_from_slice(&crc32(&inner).to_le_bytes());
                Ok(out)
            }
        }
    }

    /// Parse and fully validate one complete frame. `bytes` must be exactly one frame.
    ///
    /// Checks, in order: start of frame, minimum size, declared length equals supplied length,
    /// header checksum (and WHOOP 5 format byte), CRC-32 of the inner record.
    pub fn decode(bytes: &[u8], family: DeviceFamily) -> Result<Frame, FrameError> {
        if bytes.first() != Some(&SOF) {
            return Err(FrameError::NoStartOfFrame);
        }
        let min = family.min_frame_len();
        if bytes.len() < min {
            return Err(FrameError::TooShort { have: bytes.len(), need: min });
        }
        let declared_total = declared_total_len(bytes, family).expect("length >= minimum");
        if declared_total < min {
            return Err(FrameError::DeclaredTooSmall { declared_total });
        }
        if declared_total != bytes.len() {
            return Err(FrameError::LengthMismatch { declared_total, actual: bytes.len() });
        }
        let (inner, header) = match family {
            DeviceFamily::Whoop4 => {
                let want = crc8(&bytes[1..3]);
                if want != bytes[3] {
                    return Err(FrameError::HeaderCrc { expected: want as u16, actual: bytes[3] as u16 });
                }
                (&bytes[4..bytes.len() - 4], None)
            }
            DeviceFamily::Whoop5 => {
                let want = crc16_modbus(&bytes[0..6]);
                let got = u16::from_le_bytes([bytes[6], bytes[7]]);
                if want != got {
                    return Err(FrameError::HeaderCrc { expected: want, actual: got });
                }
                if bytes[1] != W5_FORMAT_1 {
                    return Err(FrameError::UnsupportedFormat(bytes[1]));
                }
                (&bytes[8..bytes.len() - 4], Some([bytes[4], bytes[5]]))
            }
        };
        let want = crc32(inner);
        let n = bytes.len();
        let got = u32::from_le_bytes([bytes[n - 4], bytes[n - 3], bytes[n - 2], bytes[n - 1]]);
        if want != got {
            return Err(FrameError::PayloadCrc { expected: want, actual: got });
        }
        // WHOOP 5 accepts a 13-byte frame whose inner record is a single byte, so index defensively.
        let at = |i: usize| inner.get(i).copied().unwrap_or(0);
        let payload = if inner.len() > 3 { inner[3..].to_vec() } else { Vec::new() };
        Ok(Frame { packet_type: at(0), seq: at(1), cmd: at(2), payload, header })
    }
}

/// Total on-wire size declared by a frame prefix, if enough bytes (4) are present to read the
/// length field. Does not validate anything.
pub fn declared_total_len(bytes: &[u8], family: DeviceFamily) -> Option<usize> {
    if bytes.len() < 4 {
        return None;
    }
    Some(match family {
        DeviceFamily::Whoop4 => u16::from_le_bytes([bytes[1], bytes[2]]) as usize + 4,
        DeviceFamily::Whoop5 => u16::from_le_bytes([bytes[2], bytes[3]]) as usize + 8,
    })
}
