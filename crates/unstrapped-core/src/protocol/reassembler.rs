//! Streaming reassembly of BLE notification fragments into validated frames, and the inverse
//! [`fragment`] helper the emulator uses.
//!
//! A frame can arrive split over many notifications and several frames can arrive in one
//! notification. Fragments of different characteristics are separate byte streams, so keep **one
//! [`Reassembler`] per notify characteristic** (or use [`ReassemblerSet`], which does that for you).
//!
//! Resynchronisation policy:
//! * Bytes before the first `0xAA` are discarded.
//! * A `0xAA` is accepted as a frame start only if the header checksum over its length field passes
//!   (CRC-8 on WHOOP 4, CRC-16/MODBUS + format byte on WHOOP 5) and the declared size is within
//!   `[min, MAX_FRAME_BYTES]`. Otherwise that one byte is dropped and scanning continues, so a stray
//!   `0xAA` inside payload data can never swallow the valid frames behind it for long.
//! * If the declared number of bytes has arrived but the CRC-32 fails, the whole span is reported
//!   once as [`Reassembled::Corrupt`] (so the caller can archive evidence) and scanning resumes at
//!   the next byte, which re-finds any valid frame that was misaligned inside the corrupt span.

use super::DeviceFamily;
use super::crc::{crc8, crc16_modbus};
use super::frame::{Frame, FrameError, MAX_FRAME_BYTES, SOF, W5_FORMAT_1, declared_total_len};
use super::gatt::CharacteristicRole;
use std::collections::HashMap;

/// One item produced by [`Reassembler::push`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reassembled {
    /// A frame that passed every check.
    Frame {
        /// Parsed frame.
        frame: Frame,
        /// Exact on-wire bytes (persist these before acknowledging history).
        raw: Vec<u8>,
    },
    /// A span whose header and length were plausible but whose CRC-32 failed.
    Corrupt {
        /// The bytes that were examined.
        raw: Vec<u8>,
        /// Why it was rejected (always [`FrameError::PayloadCrc`] today).
        error: FrameError,
    },
}

/// Counters describing what the reassembler discarded. Monotonic for its lifetime.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReassemblerStats {
    /// Frames emitted.
    pub frames: u64,
    /// Bytes dropped because they were not part of any frame (garbage before an SOF, false SOFs).
    pub dropped_bytes: u64,
    /// `0xAA` bytes rejected because the header checksum failed.
    pub header_crc_drops: u64,
    /// `0xAA` bytes rejected because the declared length was out of range.
    pub length_drops: u64,
    /// Complete spans rejected by the CRC-32.
    pub corrupt_frames: u64,
}

/// Reassembles one notification stream (one characteristic) into frames.
#[derive(Debug, Clone)]
pub struct Reassembler {
    family: DeviceFamily,
    buf: Vec<u8>,
    stats: ReassemblerStats,
}

impl Reassembler {
    /// New empty reassembler for `family`.
    pub fn new(family: DeviceFamily) -> Self {
        Reassembler { family, buf: Vec::new(), stats: ReassemblerStats::default() }
    }

    /// Family this reassembler parses.
    pub fn family(&self) -> DeviceFamily {
        self.family
    }

    /// Bytes currently buffered while waiting for the rest of a frame.
    pub fn buffered_len(&self) -> usize {
        self.buf.len()
    }

    /// Discard-counters.
    pub fn stats(&self) -> ReassemblerStats {
        self.stats
    }

    /// Drop any partial frame (call on disconnect: fragment buffers are connection scoped).
    pub fn reset(&mut self) {
        self.buf.clear();
    }

    /// Feed one notification and return everything that completed, in order.
    pub fn push(&mut self, fragment: &[u8]) -> Vec<Reassembled> {
        self.buf.extend_from_slice(fragment);
        let mut out = Vec::new();
        let mut head = 0usize; // consumed prefix of buf
        loop {
            // Find the next start-of-frame.
            let Some(rel) = self.buf[head..].iter().position(|&b| b == SOF) else {
                self.stats.dropped_bytes += (self.buf.len() - head) as u64;
                head = self.buf.len();
                break;
            };
            self.stats.dropped_bytes += rel as u64;
            head += rel;
            let avail = self.buf.len() - head;
            let need_header = self.family.header_len();
            if avail < need_header {
                break; // wait for the rest of the header
            }
            let b = &self.buf[head..];
            let header_ok = match self.family {
                DeviceFamily::Whoop4 => crc8(&b[1..3]) == b[3],
                DeviceFamily::Whoop5 => {
                    crc16_modbus(&b[0..6]) == u16::from_le_bytes([b[6], b[7]]) && b[1] == W5_FORMAT_1
                }
            };
            if !header_ok {
                self.stats.header_crc_drops += 1;
                self.stats.dropped_bytes += 1;
                head += 1;
                continue;
            }
            let total = declared_total_len(b, self.family).expect("header present");
            if total < self.family.min_frame_len() || total > MAX_FRAME_BYTES {
                self.stats.length_drops += 1;
                self.stats.dropped_bytes += 1;
                head += 1;
                continue;
            }
            if avail < total {
                break; // wait for the rest of the frame
            }
            let raw = b[..total].to_vec();
            match Frame::decode(&raw, self.family) {
                Ok(frame) => {
                    self.stats.frames += 1;
                    out.push(Reassembled::Frame { frame, raw });
                    head += total;
                }
                Err(error) => {
                    self.stats.corrupt_frames += 1;
                    out.push(Reassembled::Corrupt { raw, error });
                    self.stats.dropped_bytes += 1;
                    head += 1; // rescan inside the span
                }
            }
        }
        self.buf.drain(..head);
        out
    }

    /// Like [`push`](Self::push) but returns only valid frames (corrupt spans are counted in
    /// [`stats`](Self::stats) and discarded).
    pub fn push_frames(&mut self, fragment: &[u8]) -> Vec<Frame> {
        self.push(fragment)
            .into_iter()
            .filter_map(|r| match r {
                Reassembled::Frame { frame, .. } => Some(frame),
                Reassembled::Corrupt { .. } => None,
            })
            .collect()
    }
}

/// One [`Reassembler`] per notify characteristic of a family.
#[derive(Debug, Clone)]
pub struct ReassemblerSet {
    family: DeviceFamily,
    by_role: HashMap<CharacteristicRole, Reassembler>,
}

impl ReassemblerSet {
    /// Empty set; reassemblers are created lazily per role.
    pub fn new(family: DeviceFamily) -> Self {
        ReassemblerSet { family, by_role: HashMap::new() }
    }

    /// Feed a notification that arrived on the characteristic with `role`.
    pub fn push(&mut self, role: CharacteristicRole, fragment: &[u8]) -> Vec<Reassembled> {
        let family = self.family;
        self.by_role.entry(role).or_insert_with(|| Reassembler::new(family)).push(fragment)
    }

    /// Feed a notification identified by characteristic UUID; `None` if the UUID is not part of
    /// this family's service.
    pub fn push_uuid(&mut self, uuid: &str, fragment: &[u8]) -> Option<Vec<Reassembled>> {
        let role = self.family.role_of_uuid(uuid)?;
        Some(self.push(role, fragment))
    }

    /// Drop every partial frame (call on disconnect).
    pub fn reset(&mut self) {
        self.by_role.clear();
    }
}

/// Split `frame` into notification payloads for an ATT MTU of `att_mtu` (each notification carries
/// `att_mtu - 3` bytes, minimum 1). An empty frame yields no fragments.
pub fn fragment(frame: &[u8], att_mtu: usize) -> Vec<Vec<u8>> {
    fragment_by(frame, att_mtu.saturating_sub(3).max(1))
}

/// Split `frame` into chunks of at most `chunk_len` bytes (minimum 1).
pub fn fragment_by(frame: &[u8], chunk_len: usize) -> Vec<Vec<u8>> {
    frame.chunks(chunk_len.max(1)).map(<[u8]>::to_vec).collect()
}
