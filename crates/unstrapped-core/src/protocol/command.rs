//! Host commands (type 35) and strap command responses (type 36).
//!
//! Every [`HostCommand`] can be encoded (host side) and decoded again (emulator side). Command
//! numbers differ per family for clock and hello (WHOOP 5 uses the high-numbered revision-1 forms
//! 145/146/147); the rest share numbers.
//!
//! Response layout (inner record): `[36][strap seq][cmd][origin seq][result][body…]`. The strap's own
//! sequence is **not** the correlation key; match `(cmd, origin_seq)` against the request. One
//! request can yield a `Pending` response followed by a final one, and the same final response can
//! repeat, so tolerate duplicates.

use super::DeviceFamily;
use super::frame::{Frame, packet_type};
use super::util::*;
use serde::{Deserialize, Serialize};

/// Command numbers (`inner[2]` of a type-35 frame). Names follow NOOP's schema.
pub mod command_number {
    /// Toggle live heart-rate notifications.
    pub const TOGGLE_REALTIME_HR: u8 = 3;
    /// WHOOP 4: firmware version info (68-byte body).
    pub const REPORT_VERSION_INFO: u8 = 7;
    /// WHOOP 4 set clock.
    pub const SET_CLOCK: u8 = 10;
    /// WHOOP 4 get clock.
    pub const GET_CLOCK: u8 = 11;
    /// Stop streaming history without acknowledging or trimming.
    pub const ABORT_HISTORICAL_TRANSMITS: u8 = 20;
    /// Start the history offload.
    pub const SEND_HISTORICAL_DATA: u8 = 22;
    /// Acknowledge a `HISTORY_END` chunk (may let the strap reclaim storage).
    pub const HISTORICAL_DATA_RESULT: u8 = 23;
    /// Battery level.
    pub const GET_BATTERY_LEVEL: u8 = 26;
    /// Reboot the strap (not exposed as a [`HostCommand`](super::HostCommand)).
    pub const REBOOT_STRAP: u8 = 29;
    /// Range of stored history.
    pub const GET_DATA_RANGE: u8 = 34;
    /// WHOOP 4 hello (131-byte body, contains serial and key material).
    pub const GET_HELLO_HARVARD: u8 = 35;
    /// WHOOP 4 extended battery info (25-byte cached record).
    pub const GET_EXTENDED_BATTERY_INFO: u8 = 98;
    /// WHOOP 5 hello (revision 1 or 3).
    pub const GET_HELLO: u8 = 145;
    /// WHOOP 5 set clock (revision 1).
    pub const SET_CLOCK_W5: u8 = 146;
    /// WHOOP 5 get clock (revision 1).
    pub const GET_CLOCK_W5: u8 = 147;
}

/// Result byte of a command response (`payload[1]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandResult {
    /// 0: refused / failed.
    Failure,
    /// 1: accepted at that command's response boundary (not proof of asynchronous completion).
    Success,
    /// 2: pending; a later response carries the outcome.
    Pending,
    /// 3: command not supported in this context.
    Unsupported,
    /// Any other value, preserved verbatim.
    Other(u8),
}

impl CommandResult {
    /// From the wire byte.
    pub fn from_u8(b: u8) -> Self {
        match b {
            0 => CommandResult::Failure,
            1 => CommandResult::Success,
            2 => CommandResult::Pending,
            3 => CommandResult::Unsupported,
            o => CommandResult::Other(o),
        }
    }

    /// To the wire byte.
    pub fn to_u8(self) -> u8 {
        match self {
            CommandResult::Failure => 0,
            CommandResult::Success => 1,
            CommandResult::Pending => 2,
            CommandResult::Unsupported => 3,
            CommandResult::Other(o) => o,
        }
    }
}

/// Wrapping 8-bit sequence counter for host commands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SeqCounter(u8);

impl SeqCounter {
    /// Counter starting at `start`.
    pub fn new(start: u8) -> Self {
        SeqCounter(start)
    }

    /// Return the current value and advance.
    pub fn next_seq(&mut self) -> u8 {
        let v = self.0;
        self.0 = self.0.wrapping_add(1);
        v
    }
}

/// A command the host sends to the strap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostCommand {
    /// Switch live heart-rate (type 40) notifications on or off. Command 3, body `[on]`.
    ToggleRealtimeHr(bool),
    /// Read the battery. Command 26, body `[0]`.
    GetBatteryLevel,
    /// Read the strap clock. WHOOP 4: command 11 body `[0]`; WHOOP 5: command 147 body `[1]`.
    GetClock,
    /// Set the strap clock. WHOOP 4: command 10 body `unix u32 LE + 4 zero bytes`.
    /// WHOOP 5: command 146 body `[1] + unix u32 LE + ticks u16 LE` (ticks of 1/32768 s). Read the
    /// clock back afterwards; on some WHOOP 4 units one of the two documented forms latches.
    SetClock {
        /// Unix seconds.
        unix: u32,
        /// Fractional ticks (1/32768 s); WHOOP 5 only, ignored on WHOOP 4.
        subsec_ticks: u16,
    },
    /// Read the stored-history range. Command 34, body `[0]`.
    GetDataRange,
    /// Begin the history offload. Command 22, body `[0]`. The response only acknowledges the
    /// request; records then stream as type 47 between metadata START and COMPLETE.
    SendHistoricalData,
    /// Acknowledge a chunk. Command 23, body `[1] + ack_token` (the token from `HISTORY_END`,
    /// verbatim). **Send only after the chunk is durably stored.**
    HistoricalDataResult {
        /// Token copied from the `HISTORY_END` ([`HistoryEnd::ack_token`](super::HistoryEnd)).
        ack_token: Vec<u8>,
    },
    /// Stop an offload without trimming. Command 20, body `[0]`.
    AbortHistoricalTransmits,
    /// Hello / identity. WHOOP 4: command 35 body `[0]`; WHOOP 5: command 145 body `[1]` (this is
    /// also the fixed `CLIENT_HELLO` when sent with sequence 1).
    GetHello,
    /// WHOOP 4 firmware versions. Command 7, body `[0]`. Not available on WHOOP 5.
    GetVersionInfo,
    /// WHOOP 4 extended battery record. Command 98, body `[0]`. Not available on WHOOP 5.
    GetExtendedBatteryInfo,
    /// Any other command, sent verbatim.
    Raw {
        /// Command number.
        cmd: u8,
        /// Body.
        payload: Vec<u8>,
    },
}

impl HostCommand {
    /// Wire command number for `family`.
    pub fn command_number(&self, family: DeviceFamily) -> Result<u8, PacketError> {
        use command_number as n;
        let w5 = family == DeviceFamily::Whoop5;
        Ok(match self {
            HostCommand::ToggleRealtimeHr(_) => n::TOGGLE_REALTIME_HR,
            HostCommand::GetBatteryLevel => n::GET_BATTERY_LEVEL,
            HostCommand::GetClock => if w5 { n::GET_CLOCK_W5 } else { n::GET_CLOCK },
            HostCommand::SetClock { .. } => if w5 { n::SET_CLOCK_W5 } else { n::SET_CLOCK },
            HostCommand::GetDataRange => n::GET_DATA_RANGE,
            HostCommand::SendHistoricalData => n::SEND_HISTORICAL_DATA,
            HostCommand::HistoricalDataResult { .. } => n::HISTORICAL_DATA_RESULT,
            HostCommand::AbortHistoricalTransmits => n::ABORT_HISTORICAL_TRANSMITS,
            HostCommand::GetHello => if w5 { n::GET_HELLO } else { n::GET_HELLO_HARVARD },
            HostCommand::GetVersionInfo => {
                if w5 {
                    return Err(PacketError::Unsupported("REPORT_VERSION_INFO is WHOOP 4 only"));
                }
                n::REPORT_VERSION_INFO
            }
            HostCommand::GetExtendedBatteryInfo => {
                if w5 {
                    return Err(PacketError::Unsupported("GET_EXTENDED_BATTERY_INFO is WHOOP 4 only"));
                }
                n::GET_EXTENDED_BATTERY_INFO
            }
            HostCommand::Raw { cmd, .. } => *cmd,
        })
    }

    /// Body bytes (everything after the command number) for `family`.
    pub fn payload(&self, family: DeviceFamily) -> Vec<u8> {
        let w5 = family == DeviceFamily::Whoop5;
        match self {
            HostCommand::ToggleRealtimeHr(on) => vec![*on as u8],
            HostCommand::GetBatteryLevel
            | HostCommand::GetDataRange
            | HostCommand::SendHistoricalData
            | HostCommand::AbortHistoricalTransmits
            | HostCommand::GetVersionInfo
            | HostCommand::GetExtendedBatteryInfo => vec![0],
            HostCommand::GetClock => vec![if w5 { 1 } else { 0 }],
            HostCommand::GetHello => vec![if w5 { 1 } else { 0 }],
            HostCommand::SetClock { unix, subsec_ticks } => {
                if w5 {
                    let mut p = vec![1];
                    p.extend_from_slice(&unix.to_le_bytes());
                    p.extend_from_slice(&subsec_ticks.to_le_bytes());
                    p
                } else {
                    let mut p = unix.to_le_bytes().to_vec();
                    p.extend_from_slice(&[0, 0, 0, 0]);
                    p
                }
            }
            HostCommand::HistoricalDataResult { ack_token } => {
                let mut p = vec![1];
                p.extend_from_slice(ack_token);
                p
            }
            HostCommand::Raw { payload, .. } => payload.clone(),
        }
    }

    /// Build the type-35 [`Frame`] with sequence `seq`.
    pub fn to_frame(&self, family: DeviceFamily, seq: u8) -> Result<Frame, PacketError> {
        Ok(Frame::new(packet_type::COMMAND, seq, self.command_number(family)?, self.payload(family)))
    }

    /// Encode to on-wire bytes (checksums, padding included).
    pub fn encode(&self, family: DeviceFamily, seq: u8) -> Result<Vec<u8>, PacketError> {
        Ok(self.to_frame(family, seq)?.encode(family)?)
    }

    /// Interpret a received type-35 frame (emulator side). Unknown commands become
    /// [`HostCommand::Raw`]. WHOOP 5 bodies may carry trailing zero padding; [`HostCommand::Raw`] keeps it.
    pub fn from_frame(frame: &Frame, family: DeviceFamily) -> Result<HostCommand, PacketError> {
        use command_number as n;
        if !matches!(frame.packet_type, packet_type::COMMAND | packet_type::PUFFIN_COMMAND) {
            return Err(PacketError::WrongPacketType { expected: packet_type::COMMAND, got: frame.packet_type });
        }
        let w5 = family == DeviceFamily::Whoop5;
        let p = &frame.payload;
        let first = p.first().copied().unwrap_or(0);
        Ok(match (frame.cmd, w5) {
            (n::TOGGLE_REALTIME_HR, _) => HostCommand::ToggleRealtimeHr(first != 0),
            (n::GET_BATTERY_LEVEL, _) => HostCommand::GetBatteryLevel,
            (n::GET_DATA_RANGE, _) => HostCommand::GetDataRange,
            (n::SEND_HISTORICAL_DATA, _) => HostCommand::SendHistoricalData,
            (n::ABORT_HISTORICAL_TRANSMITS, _) => HostCommand::AbortHistoricalTransmits,
            (n::GET_CLOCK, false) | (n::GET_CLOCK_W5, true) => HostCommand::GetClock,
            (n::GET_HELLO_HARVARD, false) | (n::GET_HELLO, true) => HostCommand::GetHello,
            (n::REPORT_VERSION_INFO, false) => HostCommand::GetVersionInfo,
            (n::GET_EXTENDED_BATTERY_INFO, false) => HostCommand::GetExtendedBatteryInfo,
            (n::SET_CLOCK, false) => {
                need("SET_CLOCK", &frame.inner(), 3 + 4)?;
                HostCommand::SetClock { unix: u32_at(p, 0), subsec_ticks: 0 }
            }
            (n::SET_CLOCK_W5, true) => {
                need("SET_CLOCK", &frame.inner(), 3 + 7)?;
                HostCommand::SetClock { unix: u32_at(p, 1), subsec_ticks: u16_at(p, 5) }
            }
            (n::HISTORICAL_DATA_RESULT, _) => {
                if p.len() < 5 {
                    return Err(PacketError::Truncated {
                        what: "HISTORICAL_DATA_RESULT",
                        need: 3 + 5,
                        have: 3 + p.len(),
                    });
                }
                // Token is 8 bytes after the leading 0x01; WHOOP 5 padding can only add zeros after it.
                let end = p.len().min(9);
                HostCommand::HistoricalDataResult { ack_token: p[1..end].to_vec() }
            }
            (cmd, _) => HostCommand::Raw { cmd, payload: p.clone() },
        })
    }
}

/// A strap reply to a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResponse {
    /// The strap's own sequence byte (advances per response; not a correlation key).
    pub seq: u8,
    /// Command being answered.
    pub cmd: u8,
    /// Sequence byte of the request being answered (the correlation key with `cmd`).
    pub origin_seq: u8,
    /// Result byte.
    pub result: CommandResult,
    /// Command-specific body. WHOOP 5 bodies may include trailing zero padding.
    pub body: Vec<u8>,
}

impl CommandResponse {
    /// Success reply to `request` carrying `body`.
    pub fn success(strap_seq: u8, cmd: u8, origin_seq: u8, body: Vec<u8>) -> Self {
        CommandResponse { seq: strap_seq, cmd, origin_seq, result: CommandResult::Success, body }
    }

    /// Decode a type-36 / type-38 frame.
    pub fn from_frame(frame: &Frame) -> Result<Self, PacketError> {
        if !matches!(frame.packet_type, packet_type::COMMAND_RESPONSE | packet_type::PUFFIN_COMMAND_RESPONSE) {
            return Err(PacketError::WrongPacketType {
                expected: packet_type::COMMAND_RESPONSE,
                got: frame.packet_type,
            });
        }
        if frame.payload.len() < 2 {
            return Err(PacketError::Truncated {
                what: "command response",
                need: 5,
                have: 3 + frame.payload.len(),
            });
        }
        Ok(CommandResponse {
            seq: frame.seq,
            cmd: frame.cmd,
            origin_seq: frame.payload[0],
            result: CommandResult::from_u8(frame.payload[1]),
            body: frame.payload[2..].to_vec(),
        })
    }

    /// Build the type-36 [`Frame`].
    pub fn to_frame(&self) -> Frame {
        let mut p = vec![self.origin_seq, self.result.to_u8()];
        p.extend_from_slice(&self.body);
        Frame::new(packet_type::COMMAND_RESPONSE, self.seq, self.cmd, p)
    }

    /// Interpret [`body`](Self::body) for the answered command, if it is one this crate knows.
    pub fn typed_body(&self, family: DeviceFamily) -> Result<ResponseBody, PacketError> {
        ResponseBody::decode(self.cmd, family, &self.body)
    }
}

/// Clock reading from a `GET_CLOCK` response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockReading {
    /// Unix seconds. Zero can mean "clock unset" even on success.
    pub unix: u32,
    /// Fractional ticks of 1/32768 s (WHOOP 5 only; `None` on WHOOP 4).
    pub subsec_ticks: Option<u16>,
}

/// Body of a `GET_DATA_RANGE` response (WHOOP 4 and WHOOP 5 share this layout: revision 1 then 16 u32).
///
/// Positions are ring-buffer *page slots*, not timestamps. Roles marked "unresolved" are NOOP's
/// words, kept as raw numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataRange {
    /// Ring boundary A (role unresolved; 0 on a WHOOP 4 capture).
    pub boundary_a: u32,
    /// Read-page cursor.
    pub read_page: u32,
    /// Write-page cursor.
    pub write_page: u32,
    /// Acknowledged (trim) boundary.
    pub trim_page: u32,
    /// Write-wrap state / count.
    pub wrap: u32,
    /// Ring capacity in page slots (131072 in every capture).
    pub capacity_pages: u32,
    /// Record-count estimate (about 15 per page; not exact).
    pub record_estimate: u32,
    /// Derived backlog quantity (15 x pages between trim and write).
    pub derived_backlog: u32,
    /// Four `(seconds, companion)` clock pairs: boundary-A page, trim-side page, read-side page,
    /// last-written page. Seconds below 1.6e9 (for example 1262044800) or `0xFFFFFFFF` are sentinels.
    pub clocks: [(u32, u32); 4],
}

impl DataRange {
    /// Body length without padding: revision byte plus 16 u32.
    pub const BODY_LEN: usize = 65;

    /// Unix seconds if `secs` is a plausible time (2020-09 .. 2030-03), else `None`.
    pub fn plausible_unix(secs: u32) -> Option<u32> {
        (1_600_000_000..=1_900_000_000).contains(&secs).then_some(secs)
    }

    /// Timestamp of the oldest unacknowledged data (trim-side page), if plausible.
    pub fn oldest_unix(&self) -> Option<u32> {
        Self::plausible_unix(self.clocks[1].0)
    }

    /// Timestamp of the newest written page, if plausible.
    pub fn newest_unix(&self) -> Option<u32> {
        Self::plausible_unix(self.clocks[3].0)
    }

    /// Pages waiting to be read, accounting for ring wrap (`write - trim`, modulo capacity).
    /// `None` if the cursors are not inside the capacity.
    pub fn pages_behind(&self) -> Option<u32> {
        let (w, t, cap) = (self.write_page, self.trim_page, self.capacity_pages);
        (cap > 0 && w < cap && t < cap).then(|| if w >= t { w - t } else { w + (cap - t) })
    }

    fn decode(body: &[u8]) -> Result<Self, PacketError> {
        if body.len() < Self::BODY_LEN {
            return Err(PacketError::Truncated { what: "GET_DATA_RANGE body", need: Self::BODY_LEN, have: body.len() });
        }
        let w = |i: usize| u32_at(body, 1 + 4 * i);
        Ok(DataRange {
            boundary_a: w(0),
            read_page: w(1),
            write_page: w(2),
            trim_page: w(3),
            wrap: w(4),
            capacity_pages: w(5),
            record_estimate: w(6),
            derived_backlog: w(7),
            clocks: [(w(8), w(9)), (w(10), w(11)), (w(12), w(13)), (w(14), w(15))],
        })
    }

    fn encode(&self, family: DeviceFamily) -> Vec<u8> {
        // A real WHOOP 4 capture carries two bytes beyond the 65 documented; WHOOP 5 gets its two
        // from frame padding. Encode 67 for WHOOP 4 so frames match that capture byte for byte.
        let mut b = vec![0u8; if family == DeviceFamily::Whoop4 { 67 } else { Self::BODY_LEN }];
        b[0] = 1;
        let words = [
            self.boundary_a, self.read_page, self.write_page, self.trim_page, self.wrap,
            self.capacity_pages, self.record_estimate, self.derived_backlog,
            self.clocks[0].0, self.clocks[0].1, self.clocks[1].0, self.clocks[1].1,
            self.clocks[2].0, self.clocks[2].1, self.clocks[3].0, self.clocks[3].1,
        ];
        for (i, v) in words.into_iter().enumerate() {
            put_u32(&mut b, 1 + 4 * i, v);
        }
        b
    }
}

/// WHOOP 4 `REPORT_VERSION_INFO` body: two firmware version quadruples.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionInfo {
    /// "Harvard" (MAXIM application) version `a.b.c.d`, e.g. 41.17.6.0.
    pub harvard: [u32; 4],
    /// "Boylston" (Nordic) version `a.b.c.d`, e.g. 17.2.2.0.
    pub boylston: [u32; 4],
}

/// Typed command-response body.
#[derive(Debug, Clone, PartialEq)]
pub enum ResponseBody {
    /// `GET_BATTERY_LEVEL`: percent. WHOOP 4 sends `u16 LE / 10`; WHOOP 5 sends `u32 LE` whole percent.
    Battery {
        /// State of charge, 0..=100.
        percent: f32,
    },
    /// `GET_CLOCK`.
    Clock(ClockReading),
    /// `GET_DATA_RANGE`.
    DataRange(DataRange),
    /// WHOOP 4 `REPORT_VERSION_INFO`.
    VersionInfo(VersionInfo),
    /// WHOOP 4 `GET_EXTENDED_BATTERY_INFO`: pack voltage in millivolts (third `u16`, body offset 5).
    ExtendedBattery {
        /// Pack millivolts.
        millivolts: u16,
    },
    /// A command this crate has no typed body for; bytes preserved.
    Raw(Vec<u8>),
}

impl ResponseBody {
    /// Decode the body of a response to command `cmd`.
    pub fn decode(cmd: u8, family: DeviceFamily, body: &[u8]) -> Result<Self, PacketError> {
        use command_number as n;
        let w5 = family == DeviceFamily::Whoop5;
        Ok(match (cmd, w5) {
            (n::GET_BATTERY_LEVEL, false) => {
                chk(body, 2, "battery body")?;
                ResponseBody::Battery { percent: u16_at(body, 0) as f32 / 10.0 }
            }
            (n::GET_BATTERY_LEVEL, true) => {
                chk(body, 4, "battery body")?;
                ResponseBody::Battery { percent: u32_at(body, 0) as f32 }
            }
            (n::GET_CLOCK, false) => {
                chk(body, 4, "clock body")?;
                ResponseBody::Clock(ClockReading { unix: u32_at(body, 0), subsec_ticks: None })
            }
            (n::GET_CLOCK_W5, true) => {
                chk(body, 7, "clock body")?;
                ResponseBody::Clock(ClockReading { unix: u32_at(body, 1), subsec_ticks: Some(u16_at(body, 5)) })
            }
            (n::GET_DATA_RANGE, _) => ResponseBody::DataRange(DataRange::decode(body)?),
            (n::REPORT_VERSION_INFO, false) => {
                chk(body, 33, "version info body")?;
                let q = |o: usize| [0, 1, 2, 3].map(|i| u32_at(body, o + 4 * i));
                ResponseBody::VersionInfo(VersionInfo { harvard: q(1), boylston: q(17) })
            }
            (n::GET_EXTENDED_BATTERY_INFO, false) => {
                chk(body, 7, "extended battery body")?;
                ResponseBody::ExtendedBattery { millivolts: u16_at(body, 5) }
            }
            _ => ResponseBody::Raw(body.to_vec()),
        })
    }

    /// Encode to body bytes for `family` (the inverse of [`decode`](Self::decode)).
    pub fn encode(&self, family: DeviceFamily) -> Vec<u8> {
        let w5 = family == DeviceFamily::Whoop5;
        match self {
            ResponseBody::Battery { percent } => {
                if w5 {
                    (percent.round().clamp(0.0, 100.0) as u32).to_le_bytes().to_vec()
                } else {
                    let mut b = ((percent * 10.0).round().clamp(0.0, 1000.0) as u16).to_le_bytes().to_vec();
                    b.extend_from_slice(&[0, 0, 0, 0]); // trailing bytes real WHOOP 4 straps send (per NOOP's notes); meaning unknown
                    b
                }
            }
            ResponseBody::Clock(c) => {
                if w5 {
                    let mut b = vec![1];
                    b.extend_from_slice(&c.unix.to_le_bytes());
                    b.extend_from_slice(&c.subsec_ticks.unwrap_or(0).to_le_bytes());
                    b
                } else {
                    c.unix.to_le_bytes().to_vec()
                }
            }
            ResponseBody::DataRange(d) => d.encode(family),
            ResponseBody::VersionInfo(v) => {
                let mut b = vec![0u8; 68];
                b[0] = 1;
                for i in 0..4 {
                    put_u32(&mut b, 1 + 4 * i, v.harvard[i]);
                    put_u32(&mut b, 17 + 4 * i, v.boylston[i]);
                }
                b
            }
            ResponseBody::ExtendedBattery { millivolts } => {
                let mut b = vec![0u8; 25];
                put_u16(&mut b, 5, *millivolts);
                b
            }
            ResponseBody::Raw(b) => b.clone(),
        }
    }
}

fn chk(body: &[u8], len: usize, what: &'static str) -> Result<(), PacketError> {
    if body.len() < len {
        Err(PacketError::Truncated { what, need: len, have: body.len() })
    } else {
        Ok(())
    }
}
