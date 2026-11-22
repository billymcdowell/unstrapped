//! Historical records (packet type 47) and history metadata (packet type 49 / 56).
//!
//! # Record layouts
//!
//! All offsets below are **inner-record offsets** (`inner[0]` is the packet type; add 4 for an
//! absolute WHOOP 4 frame offset and 8 for a WHOOP 5 frame offset). `inner[1]` is the layout
//! version and `inner[2]` a marker byte.
//!
//! WHOOP 4 v24 (and v12, which shares its map), verified on a real 41.17.6.0 capture; inner length 96:
//!
//! | inner | type | field |
//! |------:|------|-------|
//! | 3  | u32 | record counter |
//! | 7  | u32 | Unix seconds |
//! | 11 | u16 | sub-second word (unit unverified) |
//! | 17 | u8  | heart rate bpm (0 = none) |
//! | 18 | u8  | R-R count (at most 5 words fit) |
//! | 19 | u16 x n | R-R intervals, milliseconds |
//! | 36/40/44 | f32 x3 | gravity vector (g) |
//! | 52/56/60 | f32 x3 | second gravity vector (same values in captures) |
//! | 64, 66 | u16 | raw SpO2-adjacent red / IR scalars (not a percentage) |
//! | 68 | u16 | skin temperature raw ADC (provisional conversion, see [`whoop4_skin_temp_c`]) |
//!
//! WHOOP 4 v25 (84-byte frame, inner 76): counter at 3, Unix seconds at 7, three signed movement
//! words at 69/71/73 read as gravity divided by 16384. No heart rate is carried.
//!
//! WHOOP 4 v5/v7/v9 (legacy): counter at 3, Unix seconds at 7, HR at 17, R-R count at 18, R-R at 19.
//!
//! WHOOP 5 R18 (inner 112, layout byte 18, marker `0x80`):
//!
//! | inner | type | field |
//! |------:|------|-------|
//! | 3  | u32 | record index |
//! | 7  | u32 | Unix seconds |
//! | 14 | u8  | heart rate bpm |
//! | 15 | u8  | R-R count (at most 4) |
//! | 16 | u16 x n | R-R (ms or 1/1024 s ticks, see [`RrUnit`]) |
//! | 33 | f32 | dynamic (gravity removed) acceleration, g |
//! | 37/41/45 | f32 x3 | gravity vector |
//! | 49 | u16 | cumulative step/motion counter |
//! | 55 | u8  | activity class (0 still, 1 walk, 2 run) |
//! | 65 | u16 | skin temperature, centidegrees Celsius |
//!
//! Anything else (WHOOP 4 other versions, WHOOP 5 R16/R17/R20/R21/R22/R26) is preserved verbatim as
//! [`HistoricalPacket::Unmapped`] so the raw bytes can be archived and re-decoded later.

use super::DeviceFamily;
use super::frame::{Frame, packet_type};
use super::util::*;
use serde::{Deserialize, Serialize};

/// Motion data attached to a history sample.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Motion {
    /// Gravity vector in g (x, y, z), if the layout carries one.
    pub gravity: Option<[f32; 3]>,
    /// Dynamic (gravity-removed) acceleration magnitude in g (WHOOP 5 R18).
    pub dynamic_accel_g: Option<f32>,
    /// Raw activity class (WHOOP 5 R18: 0 still, 1 walk, 2 run).
    pub activity: Option<u8>,
}

/// Raw red / infrared scalars from WHOOP 4 v24. **Not** an SpO2 percentage; NOOP documents no
/// conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spo2Raw {
    /// Red-labelled raw scalar.
    pub red: u16,
    /// Infrared-labelled raw scalar.
    pub ir: u16,
}

/// Family-neutral one-second history sample. Decoders produce it; the emulator encodes from it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct HistorySample {
    /// Strap timestamp as Unix milliseconds (the strap stores whole seconds). Valid only if the
    /// strap clock was set correctly; see `docs/PROTOCOL_NOTES.md`.
    pub unix_ms: i64,
    /// Heart rate in bpm; `None` when the strap reported 0 (no reading / off wrist).
    pub bpm: Option<u8>,
    /// R-R intervals in milliseconds (zero placeholders removed).
    pub rr_ms: Vec<u16>,
    /// Skin temperature in degrees Celsius. WHOOP 5: raw/100 (consistent with real captures).
    /// WHOOP 4: **provisional** linear map of an uncalibrated ADC, usable only for deviation from
    /// the wearer's own baseline.
    pub skin_temp_c: Option<f32>,
    /// Raw temperature register the layout carries (preferred by the encoder when set).
    pub skin_temp_raw: Option<u16>,
    /// Gravity / acceleration data.
    pub motion: Option<Motion>,
    /// Raw SpO2-adjacent scalars (WHOOP 4 v24 only).
    pub spo2: Option<Spo2Raw>,
    /// Cumulative step/motion counter (WHOOP 5 R18 only). Not equal to app/cloud steps.
    pub steps: Option<u32>,
}

/// A decoded type-47 record with its envelope fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryRecord {
    /// Layout version byte (`inner[1]`): 24/12/25/5/7/9 on WHOOP 4, 18 on WHOOP 5.
    pub version: u8,
    /// Marker byte (`inner[2]`).
    pub marker: u8,
    /// Record counter / index (`inner[3..7]`). Not a timestamp.
    pub index: u32,
    /// The measurements.
    pub sample: HistorySample,
}

impl HistoryRecord {
    /// Record in the family's primary layout (WHOOP 4 v24, WHOOP 5 R18).
    pub fn new(family: DeviceFamily, index: u32, sample: HistorySample) -> Self {
        match family {
            DeviceFamily::Whoop4 => HistoryRecord { version: 24, marker: 0x05, index, sample },
            DeviceFamily::Whoop5 => HistoryRecord { version: 18, marker: 0x80, index, sample },
        }
    }
}

/// Result of decoding a type-47 frame.
#[derive(Debug, Clone, PartialEq)]
pub enum HistoricalPacket {
    /// A layout this crate maps.
    Record(HistoryRecord),
    /// A layout (or length) this crate does not map. Archive the frame verbatim.
    Unmapped {
        /// Version byte (`inner[1]`).
        version: u8,
        /// The untouched frame.
        frame: Frame,
    },
}

const W4_V24_LEN: usize = 96;
const W4_V25_LEN: usize = 76;
const W5_R18_LEN: usize = 112;

/// Provisional WHOOP 4 skin-temperature map: raw 826 is 33.0 C with 0.05 C per count (NOOP's
/// two-anchor-less placeholder). The register offset differs per device, so treat the result as a
/// relative signal only.
pub fn whoop4_skin_temp_c(raw: u16) -> f32 {
    33.0 + (raw as f32 - 826.0) * 0.05
}

fn whoop4_raw_from_c(c: f32) -> u16 {
    (826.0 + (c - 33.0) / 0.05).round().clamp(0.0, u16::MAX as f32) as u16
}

fn finite3(v: [f32; 3]) -> Option<[f32; 3]> {
    v.iter().all(|x| x.is_finite()).then_some(v)
}

fn sample_secs(unix_ms: i64) -> u32 {
    (unix_ms.div_euclid(1000)).clamp(0, u32::MAX as i64) as u32
}

/// Decode a type-47 frame.
pub fn decode_historical(
    frame: &Frame,
    family: DeviceFamily,
    opts: &CodecOptions,
) -> Result<HistoricalPacket, PacketError> {
    if frame.packet_type != packet_type::HISTORICAL_DATA {
        return Err(PacketError::WrongPacketType {
            expected: packet_type::HISTORICAL_DATA,
            got: frame.packet_type,
        });
    }
    let inner = frame.inner();
    let unmapped = || Ok(HistoricalPacket::Unmapped { version: frame.seq, frame: frame.clone() });
    let rec = match (family, frame.seq) {
        (DeviceFamily::Whoop4, 24 | 12) if inner.len() >= 80 => decode_w4_v24(&inner),
        (DeviceFamily::Whoop4, 5 | 7 | 9) if inner.len() >= 19 => decode_w4_legacy(&inner),
        (DeviceFamily::Whoop4, 25) if inner.len() >= 75 => decode_w4_v25(&inner),
        (DeviceFamily::Whoop5, 18) if inner.len() >= 66 => decode_w5_r18(&inner, opts.rr_unit(family)),
        _ => return unmapped(),
    };
    Ok(HistoricalPacket::Record(rec))
}

fn base(inner: &[u8], sample: HistorySample) -> HistoryRecord {
    HistoryRecord { version: inner[1], marker: inner[2], index: u32_at(inner, 3), sample }
}

fn decode_w4_v24(inner: &[u8]) -> HistoryRecord {
    let hr = inner[17];
    let temp_raw = u16_at(inner, 68);
    let g = finite3([f32_at(inner, 36), f32_at(inner, 40), f32_at(inner, 44)]);
    let sample = HistorySample {
        unix_ms: u32_at(inner, 7) as i64 * 1000,
        bpm: (hr != 0).then_some(hr),
        rr_ms: read_rr(inner, 18, 19, 5, RrUnit::Milliseconds),
        skin_temp_c: Some(whoop4_skin_temp_c(temp_raw)),
        skin_temp_raw: Some(temp_raw),
        motion: g.map(|g| Motion { gravity: Some(g), ..Motion::default() }),
        spo2: Some(Spo2Raw { red: u16_at(inner, 64), ir: u16_at(inner, 66) }),
        steps: None,
    };
    base(inner, sample)
}

fn decode_w4_legacy(inner: &[u8]) -> HistoryRecord {
    let hr = inner[17];
    let sample = HistorySample {
        unix_ms: u32_at(inner, 7) as i64 * 1000,
        bpm: (hr != 0).then_some(hr),
        rr_ms: read_rr(inner, 18, 19, 5, RrUnit::Milliseconds),
        ..HistorySample::default()
    };
    base(inner, sample)
}

fn decode_w4_v25(inner: &[u8]) -> HistoryRecord {
    let g = [69, 71, 73].map(|o| i16_at(inner, o) as f32 / 16384.0);
    let sample = HistorySample {
        unix_ms: u32_at(inner, 7) as i64 * 1000,
        motion: Some(Motion { gravity: Some(g), ..Motion::default() }),
        ..HistorySample::default()
    };
    base(inner, sample)
}

fn decode_w5_r18(inner: &[u8], unit: RrUnit) -> HistoryRecord {
    let hr = inner[14];
    let temp_raw = u16_at(inner, 65);
    let dyn_a = f32_at(inner, 33);
    let g = finite3([f32_at(inner, 37), f32_at(inner, 41), f32_at(inner, 45)]);
    let motion = Motion {
        gravity: g,
        dynamic_accel_g: dyn_a.is_finite().then_some(dyn_a),
        activity: Some(inner[55]),
    };
    let sample = HistorySample {
        unix_ms: u32_at(inner, 7) as i64 * 1000,
        bpm: (hr != 0).then_some(hr),
        rr_ms: read_rr(inner, 15, 16, 4, unit),
        skin_temp_c: Some(temp_raw as f32 / 100.0),
        skin_temp_raw: Some(temp_raw),
        motion: Some(motion),
        spo2: None,
        steps: Some(u16_at(inner, 49) as u32),
    };
    base(inner, sample)
}

impl HistoryRecord {
    /// Build the type-47 [`Frame`] for this record.
    ///
    /// Encodable layouts: WHOOP 4 versions 24 and 12 (v24 map), 25, 5/7/9 (legacy); WHOOP 5 version 18.
    /// Fields a layout cannot carry are dropped.
    pub fn to_frame(&self, family: DeviceFamily, opts: &CodecOptions) -> Result<Frame, PacketError> {
        let s = &self.sample;
        let temp_raw = s.skin_temp_raw.or_else(|| {
            s.skin_temp_c.map(|c| match family {
                DeviceFamily::Whoop4 => whoop4_raw_from_c(c),
                DeviceFamily::Whoop5 => (c * 100.0).round().clamp(0.0, u16::MAX as f32) as u16,
            })
        });
        let len = match (family, self.version) {
            (DeviceFamily::Whoop4, 24 | 12) => W4_V24_LEN,
            (DeviceFamily::Whoop4, 25) => W4_V25_LEN,
            (DeviceFamily::Whoop4, 5 | 7 | 9) => 27 + 4,
            (DeviceFamily::Whoop5, 18) => W5_R18_LEN,
            _ => return Err(PacketError::Unsupported("record layout not encodable for this family")),
        };
        let mut inner = vec![0u8; len];
        inner[0] = packet_type::HISTORICAL_DATA;
        inner[1] = self.version;
        inner[2] = self.marker;
        put_u32(&mut inner, 3, self.index);
        put_u32(&mut inner, 7, sample_secs(s.unix_ms));
        let hr = s.bpm.unwrap_or(0);
        match (family, self.version) {
            (DeviceFamily::Whoop4, 24 | 12) => {
                inner[17] = hr;
                write_rr(&mut inner, 18, 19, 5, &s.rr_ms, RrUnit::Milliseconds);
                if let Some(g) = s.motion.and_then(|m| m.gravity) {
                    for (i, v) in g.iter().enumerate() {
                        put_f32(&mut inner, 36 + 4 * i, *v);
                        put_f32(&mut inner, 52 + 4 * i, *v);
                    }
                }
                if let Some(sp) = s.spo2 {
                    put_u16(&mut inner, 64, sp.red);
                    put_u16(&mut inner, 66, sp.ir);
                }
                put_u16(&mut inner, 68, temp_raw.unwrap_or(0));
            }
            (DeviceFamily::Whoop4, 5 | 7 | 9) => {
                inner[17] = hr;
                write_rr(&mut inner, 18, 19, 5, &s.rr_ms, RrUnit::Milliseconds);
            }
            (DeviceFamily::Whoop4, 25) => {
                if let Some(g) = s.motion.and_then(|m| m.gravity) {
                    for (i, v) in g.iter().enumerate() {
                        put_i16(&mut inner, 69 + 2 * i, (v * 16384.0).round() as i16);
                    }
                }
            }
            (DeviceFamily::Whoop5, 18) => {
                inner[14] = hr;
                write_rr(&mut inner, 15, 16, 4, &s.rr_ms, opts.rr_unit(family));
                if let Some(m) = s.motion {
                    if let Some(d) = m.dynamic_accel_g {
                        put_f32(&mut inner, 33, d);
                    }
                    if let Some(g) = m.gravity {
                        for (i, v) in g.iter().enumerate() {
                            put_f32(&mut inner, 37 + 4 * i, *v);
                        }
                    }
                    inner[55] = m.activity.unwrap_or(0);
                }
                put_u16(&mut inner, 49, s.steps.unwrap_or(0).min(u16::MAX as u32) as u16);
                put_u16(&mut inner, 65, temp_raw.unwrap_or(0));
            }
            _ => unreachable!("filtered above"),
        }
        Ok(Frame::new(inner[0], inner[1], inner[2], inner[3..].to_vec()))
    }
}

// ---------------------------------------------------------------------------------------------
// History metadata (type 49; WHOOP 5 also 56)
// ---------------------------------------------------------------------------------------------

/// Metadata type values (`inner[2]`).
pub mod meta_type {
    /// Start of an offload stream.
    pub const HISTORY_START: u8 = 1;
    /// End of a chunk: the host must persist, then acknowledge with the token.
    pub const HISTORY_END: u8 = 2;
    /// End of the whole offload attempt.
    pub const HISTORY_COMPLETE: u8 = 3;
}

/// Payload of a `HISTORY_END`: chunk boundary plus the opaque acknowledgement token.
///
/// Layout (identical on both families, starting at `inner[3]`): `unix u32`, `subsec u16`,
/// `reserved u32`, `ack_token[8]`. On WHOOP 4 the token is at absolute frame offsets 17..25 and on
/// WHOOP 5 at 21..29. The token's first word is the trim cursor, the second the write-wrap state.
/// **Echo it verbatim; never rebuild it from timestamps.**
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEnd {
    /// Record time of the chunk boundary (Unix seconds).
    pub unix: u32,
    /// Sub-second word (unit unverified).
    pub subsec: u16,
    /// Unmapped word.
    pub reserved: u32,
    /// The acknowledgement token, normally 8 bytes (4 in some synthetic fixtures).
    pub ack_token: Vec<u8>,
}

impl HistoryEnd {
    /// First word of the token: the strap's trim cursor.
    pub fn trim_cursor(&self) -> u32 {
        let mut b = [0u8; 4];
        let n = self.ack_token.len().min(4);
        b[..n].copy_from_slice(&self.ack_token[..n]);
        u32::from_le_bytes(b)
    }
}

/// A decoded metadata packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryMetadata {
    /// `HISTORY_START`. WHOOP 4 sends no payload; WHOOP 5 sends 36+ bytes beginning with a Unix time.
    Start {
        /// Strap sequence byte.
        seq: u8,
        /// Payload after the metadata type byte (opaque).
        payload: Vec<u8>,
    },
    /// `HISTORY_END`.
    End {
        /// Strap sequence byte.
        seq: u8,
        /// Chunk boundary and ack token.
        end: HistoryEnd,
    },
    /// `HISTORY_COMPLETE`.
    Complete {
        /// Strap sequence byte.
        seq: u8,
        /// Payload after the metadata type byte (opaque).
        payload: Vec<u8>,
    },
    /// A metadata type this crate does not know.
    Other {
        /// Strap sequence byte.
        seq: u8,
        /// Metadata type byte.
        meta_type: u8,
        /// Payload after the type byte.
        payload: Vec<u8>,
    },
}

impl HistoryMetadata {
    /// Convenience constructor for a `HISTORY_END`.
    pub fn end(seq: u8, end: HistoryEnd) -> Self {
        HistoryMetadata::End { seq, end }
    }

    /// Decode a type-49 / type-56 frame.
    pub fn from_frame(frame: &Frame) -> Result<Self, PacketError> {
        if !matches!(frame.packet_type, packet_type::METADATA | packet_type::PUFFIN_METADATA) {
            return Err(PacketError::WrongPacketType {
                expected: packet_type::METADATA,
                got: frame.packet_type,
            });
        }
        let seq = frame.seq;
        let p = &frame.payload;
        Ok(match frame.cmd {
            meta_type::HISTORY_START => HistoryMetadata::Start { seq, payload: p.clone() },
            meta_type::HISTORY_COMPLETE => HistoryMetadata::Complete { seq, payload: p.clone() },
            meta_type::HISTORY_END => {
                if p.len() < 14 {
                    return Err(PacketError::Truncated {
                        what: "HISTORY_END",
                        need: 3 + 14,
                        have: 3 + p.len(),
                    });
                }
                let token_end = p.len().min(18);
                HistoryMetadata::End {
                    seq,
                    end: HistoryEnd {
                        unix: u32_at(p, 0),
                        subsec: u16_at(p, 4),
                        reserved: u32_at(p, 6),
                        ack_token: p[10..token_end].to_vec(),
                    },
                }
            }
            other => HistoryMetadata::Other { seq, meta_type: other, payload: p.clone() },
        })
    }

    /// Build the type-49 [`Frame`].
    pub fn to_frame(&self) -> Frame {
        match self {
            HistoryMetadata::Start { seq, payload } => {
                Frame::new(packet_type::METADATA, *seq, meta_type::HISTORY_START, payload.clone())
            }
            HistoryMetadata::Complete { seq, payload } => {
                Frame::new(packet_type::METADATA, *seq, meta_type::HISTORY_COMPLETE, payload.clone())
            }
            HistoryMetadata::End { seq, end } => {
                let mut p = Vec::with_capacity(10 + end.ack_token.len());
                p.extend_from_slice(&end.unix.to_le_bytes());
                p.extend_from_slice(&end.subsec.to_le_bytes());
                p.extend_from_slice(&end.reserved.to_le_bytes());
                p.extend_from_slice(&end.ack_token);
                Frame::new(packet_type::METADATA, *seq, meta_type::HISTORY_END, p)
            }
            HistoryMetadata::Other { seq, meta_type, payload } => {
                Frame::new(packet_type::METADATA, *seq, *meta_type, payload.clone())
            }
        }
    }
}
