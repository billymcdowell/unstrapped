//! WHOOP BLE protocol. OWNED BY THE PROTOCOL AGENT — expand freely, but keep the items below
//! (names + meaning) stable because the emulator and the Tauri app depend on them.
//!
//! # Module map
//!
//! * [`crc`]: CRC-8, CRC-16/MODBUS, CRC-32.
//! * [`gatt`]: per-family characteristics and roles ([`DeviceFamily::characteristics`]).
//! * [`frame`]: [`Frame`] encode / decode / validation for both families.
//! * [`reassembler`]: [`Reassembler`] (fragments to frames) and [`fragment`] (frames to fragments).
//! * [`packet`]: [`Packet`] enum, realtime HR, events.
//! * [`history`]: history records ([`HistorySample`], [`HistoryRecord`]) and [`HistoryMetadata`].
//! * [`command`]: [`HostCommand`], [`CommandResponse`], [`ResponseBody`], [`DataRange`].
//! * [`sync`]: the history offload sequence and [`HistorySync`] (persist before ack).
//! * [`std_hr`]: standard `0x2A37` Heart Rate Measurement.
//!
//! Typical host loop: bytes from a notify characteristic go into a [`Reassembler`]; each
//! [`Reassembled::Frame`] is passed to [`Packet::decode`]; commands are built with
//! [`HostCommand::encode`] and written to the command characteristic. The emulator does the
//! mirror image with [`Packet::encode`], [`fragment`] and [`HostCommand::from_frame`].

pub mod command;
pub mod crc;
pub mod frame;
pub mod gatt;
pub mod history;
pub mod packet;
pub mod reassembler;
pub mod std_hr;
pub mod sync;
mod util;

#[cfg(test)]
mod test_vectors;
#[cfg(test)]
mod tests;

pub use command::{
    ClockReading, CommandResponse, CommandResult, DataRange, HostCommand, ResponseBody, SeqCounter,
    VersionInfo, command_number,
};
pub use crc::{crc8, crc16_modbus, crc32};
pub use frame::{Frame, FrameError, MAX_FRAME_BYTES, declared_total_len, packet_type};
pub use gatt::{
    Characteristic, CharacteristicRole, HeaderCrcKind, WHOOP5_CLIENT_HELLO,
};
pub use history::{
    HistoricalPacket, HistoryEnd, HistoryMetadata, HistoryRecord, HistorySample, Motion, Spo2Raw,
    meta_type,
};
pub use packet::{BatteryEvent, CommandFrame, Event, Packet, RealtimeHr, event_number};
pub use reassembler::{Reassembled, Reassembler, ReassemblerSet, ReassemblerStats, fragment, fragment_by};
pub use std_hr::StdHeartRate;
pub use sync::{HistorySync, PendingChunk, SyncEvent};
pub use util::{CodecOptions, PacketError, RrUnit};

use serde::{Deserialize, Serialize};

/// Which strap generation a connection belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceFamily {
    /// WHOOP 4.0 — CRC8 (poly 0x07) header check.
    Whoop4,
    /// WHOOP 5.0 / MG — CRC16-Modbus header check.
    Whoop5,
}

impl DeviceFamily {
    /// Advertised custom GATT service UUID (lowercase, hyphenated).
    pub fn service_uuid(self) -> &'static str {
        match self {
            DeviceFamily::Whoop4 => "61080001-8d6d-82b8-614a-1c8cb0f8dcc6",
            DeviceFamily::Whoop5 => "fd4b0001-cce1-4033-93ce-002d5875f58a",
        }
    }

    /// Family whose custom service UUID matches `uuid` (case-insensitive).
    pub fn from_service_uuid(uuid: &str) -> Option<Self> {
        let u = uuid.to_ascii_lowercase();
        [DeviceFamily::Whoop4, DeviceFamily::Whoop5]
            .into_iter()
            .find(|f| f.service_uuid() == u)
    }
}
