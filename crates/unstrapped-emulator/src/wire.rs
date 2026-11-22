//! Wire format between the emulator service and the app: newline-delimited JSON over TCP,
//! one message per line. Raw BLE bytes are lowercase hex. THIS FILE IS A SHARED CONTRACT.

use serde::{Deserialize, Serialize};

use crate::{CharacteristicInfo, DeviceFamily};

/// Host (app) → emulator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum HostMsg {
    /// Like a BLE connect. Emulator replies with `EmuMsg::Connected`.
    Connect,
    Subscribe { characteristic: String },
    Unsubscribe { characteristic: String },
    /// Like a BLE write-with/without-response.
    Write { characteristic: String, hex: String },
    Disconnect,
    /// Dev controls (not part of real BLE): e.g. "out_of_range", "in_range", "take_off", "put_on",
    /// or `advance_minutes` to jump the emulated clock forward.
    Control { action: String, #[serde(default)] value: Option<f64> },
}

/// Emulator → host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "ev", rename_all = "snake_case")]
pub enum EmuMsg {
    /// Sent immediately on TCP accept — the "advertisement".
    Advertisement {
        name: String,
        device_id: String,
        family: DeviceFamily,
        service_uuid: String,
        rssi: i16,
        in_range: bool,
    },
    Connected { characteristics: Vec<CharacteristicInfo> },
    Notify { characteristic: String, hex: String },
    Disconnected { reason: String },
    Error { message: String },
}
