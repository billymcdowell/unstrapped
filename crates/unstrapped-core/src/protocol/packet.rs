//! Typed packets: the family-neutral view of every frame this crate understands.
//!
//! Decode with [`Packet::decode`] / [`Packet::decode_with`]; encode with [`Packet::encode`] /
//! [`Packet::to_frame`]. `decode(encode(p)) == p` holds for every variant (WHOOP 5 command bodies
//! and responses can pick up trailing zero padding, see [`Frame`]).
//!
//! # Realtime heart rate (type 40), inner offsets
//!
//! | inner | type | field |
//! |------:|------|-------|
//! | 2 | u32 | device seconds (Unix when the strap clock is set) |
//! | 6 | u16 | sub-second word (unit unverified) |
//! | 8 | u8 | heart rate bpm |
//! | 9 | u8 | R-R count |
//! | 10 | u16 x n | R-R words (up to 5; milliseconds, or 1/1024 s on WHOOP 5 if [`RrUnit::Ticks1024`]) |
//!
//! The encoded inner record is always 20 bytes, matching real frames (WHOOP 4: 28-byte frame;
//! WHOOP 5: 32-byte frame).
//!
//! # Events (type 48), inner offsets
//!
//! | inner | type | field |
//! |------:|------|-------|
//! | 2 | u8 | event number ([`event_number`]) |
//! | 4 | u32 | event Unix seconds |
//! | 8.. | | event-specific payload |
//!
//! `BATTERY_LEVEL` (3) payload: `u16 LE` tenths of a percent at inner 13, `u16 LE` millivolts at 17,
//! charging flag (bit 0) at 22. Identical on both families.

use super::DeviceFamily;
use super::command::CommandResponse;
use super::frame::{Frame, packet_type};
use super::history::{HistoricalPacket, HistoryMetadata, HistoryRecord, decode_historical};
use super::util::*;
use serde::{Deserialize, Serialize};

pub use super::util::{CodecOptions, PacketError, RrUnit};

/// Live heart-rate packet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealtimeHr {
    /// Strap sequence byte.
    pub seq: u8,
    /// Device seconds (`inner[2..6]`); Unix time only if the strap clock is set.
    pub device_seconds: u32,
    /// Sub-second word (`inner[6..8]`), unit unverified.
    pub subsec: u16,
    /// Heart rate in bpm (0 means no reading).
    pub bpm: u8,
    /// R-R intervals in milliseconds (zeros removed, at most 5).
    pub rr_ms: Vec<u16>,
}

impl RealtimeHr {
    /// Device time as Unix milliseconds (assumes the strap clock holds Unix time).
    pub fn unix_ms(&self) -> i64 {
        self.device_seconds as i64 * 1000
    }
}

/// Event packet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// Strap sequence byte.
    pub seq: u8,
    /// Event number (see [`event_number`]).
    pub number: u8,
    /// Event time, Unix seconds if the strap clock is set.
    pub unix: u32,
    /// Event-specific payload (`inner[8..]`). WHOOP 5 may include trailing padding zeros.
    pub payload: Vec<u8>,
}

/// Decoded `BATTERY_LEVEL` event payload.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BatteryEvent {
    /// State of charge in percent (tenths resolution).
    pub percent: f32,
    /// Pack millivolts.
    pub millivolts: u16,
    /// Charging flag (bit 0 of the flags byte).
    pub charging: bool,
}

/// Event numbers (`EventNumber` in NOOP's schema).
pub mod event_number {
    /// Periodic battery report.
    pub const BATTERY_LEVEL: u8 = 3;
    /// Charger connected.
    pub const CHARGING_ON: u8 = 7;
    /// Charger disconnected.
    pub const CHARGING_OFF: u8 = 8;
    /// Strap put on the wrist.
    pub const WRIST_ON: u8 = 9;
    /// Strap taken off.
    pub const WRIST_OFF: u8 = 10;
    /// Double tap gesture.
    pub const DOUBLE_TAP: u8 = 14;
    /// Strap booted.
    pub const BOOT: u8 = 15;
    /// Realtime HR notifications switched on.
    pub const BLE_REALTIME_HR_ON: u8 = 33;
    /// Realtime HR notifications switched off.
    pub const BLE_REALTIME_HR_OFF: u8 = 34;
    /// Raw data collection started.
    pub const RAW_DATA_COLLECTION_ON: u8 = 46;
    /// Raw data collection stopped.
    pub const RAW_DATA_COLLECTION_OFF: u8 = 47;
    /// Extended battery information available.
    pub const EXTENDED_BATTERY_INFORMATION: u8 = 63;
    /// High-frequency sync entered.
    pub const HIGH_FREQ_SYNC_ENABLED: u8 = 97;
    /// High-frequency sync left.
    pub const HIGH_FREQ_SYNC_DISABLED: u8 = 98;

    /// NOOP's name for an event number, if it has one.
    pub fn name(n: u8) -> Option<&'static str> {
        Some(match n {
            0 => "UNDEFINED", 1 => "ERROR", 2 => "CONSOLE_OUTPUT", 3 => "BATTERY_LEVEL",
            4 => "SYSTEM_CONTROL", 5 => "EXTERNAL_5V_ON", 6 => "EXTERNAL_5V_OFF", 7 => "CHARGING_ON",
            8 => "CHARGING_OFF", 9 => "WRIST_ON", 10 => "WRIST_OFF", 11 => "BLE_CONNECTION_UP",
            12 => "BLE_CONNECTION_DOWN", 13 => "RTC_LOST", 14 => "DOUBLE_TAP", 15 => "BOOT",
            16 => "SET_RTC", 17 => "TEMPERATURE_LEVEL", 18 => "PAIRING_MODE",
            19 => "SERIAL_HEAD_CONNECTED", 20 => "SERIAL_HEAD_REMOVED", 21 => "BATTERY_PACK_CONNECTED",
            22 => "BATTERY_PACK_REMOVED", 23 => "BLE_BONDED", 24 => "BLE_HR_PROFILE_ENABLED",
            25 => "BLE_HR_PROFILE_DISABLED", 26 => "TRIM_ALL_DATA", 27 => "TRIM_ALL_DATA_ENDED",
            28 => "FLASH_INIT_COMPLETE", 29 => "STRAP_CONDITION_REPORT", 30 => "BOOT_REPORT",
            31 => "EXIT_VIRGIN_MODE", 32 => "CAPTOUCH_AUTOTHRESHOLD_ACTION", 33 => "BLE_REALTIME_HR_ON",
            34 => "BLE_REALTIME_HR_OFF", 35 => "ACCELEROMETER_RESET", 36 => "AFE_RESET",
            37 => "SHIP_MODE_ENABLED", 38 => "SHIP_MODE_DISABLED", 39 => "SHIP_MODE_BOOT",
            40 => "CH1_SATURATION_DETECTED", 41 => "CH2_SATURATION_DETECTED",
            42 => "ACCELEROMETER_SATURATION_DETECTED", 43 => "BLE_SYSTEM_RESET", 44 => "BLE_SYSTEM_ON",
            45 => "BLE_SYSTEM_INITIALIZED", 46 => "RAW_DATA_COLLECTION_ON", 47 => "RAW_DATA_COLLECTION_OFF",
            56 => "STRAP_DRIVEN_ALARM_SET", 57 => "STRAP_DRIVEN_ALARM_EXECUTED",
            58 => "APP_DRIVEN_ALARM_EXECUTED", 59 => "STRAP_DRIVEN_ALARM_DISABLED", 60 => "HAPTICS_FIRED",
            63 => "EXTENDED_BATTERY_INFORMATION", 96 => "HIGH_FREQ_SYNC_PROMPT",
            97 => "HIGH_FREQ_SYNC_ENABLED", 98 => "HIGH_FREQ_SYNC_DISABLED", 100 => "HAPTICS_TERMINATED",
            _ => return None,
        })
    }
}

impl Event {
    /// Event named `number` with no payload.
    pub fn simple(seq: u8, number: u8, unix: u32) -> Self {
        Event { seq, number, unix, payload: Vec::new() }
    }

    /// A `BATTERY_LEVEL` event with the layout real straps send (22-byte payload).
    pub fn battery(seq: u8, unix: u32, b: BatteryEvent) -> Self {
        let mut payload = vec![0u8; 22];
        // payload offsets are inner offsets minus 8
        put_u16(&mut payload, 5, (b.percent * 10.0).round().clamp(0.0, 1000.0) as u16);
        put_u16(&mut payload, 9, b.millivolts);
        payload[14] = b.charging as u8;
        Event { seq, number: event_number::BATTERY_LEVEL, unix, payload }
    }

    /// Decode the battery payload if this is a `BATTERY_LEVEL` event long enough to hold one.
    pub fn as_battery(&self) -> Option<BatteryEvent> {
        if self.number != event_number::BATTERY_LEVEL || self.payload.len() < 15 {
            return None;
        }
        Some(BatteryEvent {
            percent: u16_at(&self.payload, 5) as f32 / 10.0,
            millivolts: u16_at(&self.payload, 9),
            charging: self.payload[14] & 1 != 0,
        })
    }
}

/// A raw host command frame as seen by a receiver (strap/emulator side); see
/// [`HostCommand::from_frame`](super::HostCommand::from_frame) for the typed view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandFrame {
    /// Host sequence number.
    pub seq: u8,
    /// Command number.
    pub cmd: u8,
    /// Command body.
    pub payload: Vec<u8>,
}

/// Every packet kind this crate decodes.
#[derive(Debug, Clone, PartialEq)]
pub enum Packet {
    /// Host to strap command (type 35).
    Command(CommandFrame),
    /// Strap reply (type 36, or 38).
    CommandResponse(CommandResponse),
    /// Live heart rate (type 40).
    RealtimeHr(RealtimeHr),
    /// Historical data (type 47).
    Historical(HistoricalPacket),
    /// Strap event (type 48).
    Event(Event),
    /// History offload metadata (type 49, or 56 on WHOOP 5).
    HistoryMetadata(HistoryMetadata),
    /// Any other packet type (raw sensor 43, console logs 50, IMU 51/52, ...), preserved verbatim.
    Unknown(Frame),
}

impl Packet {
    /// Decode with default [`CodecOptions`].
    pub fn decode(frame: &Frame, family: DeviceFamily) -> Result<Packet, PacketError> {
        Self::decode_with(frame, family, &CodecOptions::default())
    }

    /// Decode a validated frame.
    pub fn decode_with(frame: &Frame, family: DeviceFamily, opts: &CodecOptions) -> Result<Packet, PacketError> {
        Ok(match frame.packet_type {
            packet_type::COMMAND | packet_type::PUFFIN_COMMAND => Packet::Command(CommandFrame {
                seq: frame.seq,
                cmd: frame.cmd,
                payload: frame.payload.clone(),
            }),
            packet_type::COMMAND_RESPONSE | packet_type::PUFFIN_COMMAND_RESPONSE => {
                Packet::CommandResponse(CommandResponse::from_frame(frame)?)
            }
            packet_type::REALTIME_DATA => Packet::RealtimeHr(decode_realtime(frame, family, opts)?),
            packet_type::HISTORICAL_DATA => Packet::Historical(decode_historical(frame, family, opts)?),
            packet_type::EVENT => Packet::Event(decode_event(frame)?),
            packet_type::METADATA | packet_type::PUFFIN_METADATA => {
                Packet::HistoryMetadata(HistoryMetadata::from_frame(frame)?)
            }
            _ => Packet::Unknown(frame.clone()),
        })
    }

    /// Build the [`Frame`] for this packet with default options.
    pub fn to_frame(&self, family: DeviceFamily) -> Result<Frame, PacketError> {
        self.to_frame_with(family, &CodecOptions::default())
    }

    /// Build the [`Frame`] for this packet.
    pub fn to_frame_with(&self, family: DeviceFamily, opts: &CodecOptions) -> Result<Frame, PacketError> {
        Ok(match self {
            Packet::Command(c) => Frame::new(packet_type::COMMAND, c.seq, c.cmd, c.payload.clone()),
            Packet::CommandResponse(r) => r.to_frame(),
            Packet::RealtimeHr(r) => encode_realtime(r, family, opts),
            Packet::Historical(HistoricalPacket::Record(r)) => r.to_frame(family, opts)?,
            Packet::Historical(HistoricalPacket::Unmapped { frame, .. }) => frame.clone(),
            Packet::Event(e) => {
                let mut p = vec![0u8]; // inner[3]: reserved
                p.extend_from_slice(&e.unix.to_le_bytes());
                p.extend_from_slice(&e.payload);
                Frame::new(packet_type::EVENT, e.seq, e.number, p)
            }
            Packet::HistoryMetadata(m) => m.to_frame(),
            Packet::Unknown(f) => f.clone(),
        })
    }

    /// Encode to on-wire bytes.
    pub fn encode(&self, family: DeviceFamily) -> Result<Vec<u8>, PacketError> {
        Ok(self.to_frame(family)?.encode(family)?)
    }

    /// Encode to on-wire bytes with explicit options.
    pub fn encode_with(&self, family: DeviceFamily, opts: &CodecOptions) -> Result<Vec<u8>, PacketError> {
        Ok(self.to_frame_with(family, opts)?.encode(family)?)
    }
}

impl From<HistoryRecord> for Packet {
    fn from(r: HistoryRecord) -> Self {
        Packet::Historical(HistoricalPacket::Record(r))
    }
}

const REALTIME_INNER_LEN: usize = 20;

fn decode_realtime(frame: &Frame, family: DeviceFamily, opts: &CodecOptions) -> Result<RealtimeHr, PacketError> {
    let inner = frame.inner();
    need("realtime HR", &inner, 10)?;
    Ok(RealtimeHr {
        seq: inner[1],
        device_seconds: u32_at(&inner, 2),
        subsec: u16_at(&inner, 6),
        bpm: inner[8],
        rr_ms: read_rr(&inner, 9, 10, 5, opts.rr_unit(family)),
    })
}

fn encode_realtime(r: &RealtimeHr, family: DeviceFamily, opts: &CodecOptions) -> Frame {
    let mut inner = vec![0u8; REALTIME_INNER_LEN];
    inner[0] = packet_type::REALTIME_DATA;
    inner[1] = r.seq;
    put_u32(&mut inner, 2, r.device_seconds);
    put_u16(&mut inner, 6, r.subsec);
    inner[8] = r.bpm;
    write_rr(&mut inner, 9, 10, 5, &r.rr_ms, opts.rr_unit(family));
    Frame::new(inner[0], inner[1], inner[2], inner[3..].to_vec())
}

fn decode_event(frame: &Frame) -> Result<Event, PacketError> {
    let inner = frame.inner();
    need("event", &inner, 8)?;
    Ok(Event { seq: inner[1], number: inner[2], unix: u32_at(&inner, 4), payload: inner[8..].to_vec() })
}
