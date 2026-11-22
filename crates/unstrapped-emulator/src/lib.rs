//! unstrapped-emulator — a stand-in WHOOP strap for development without hardware.
//!
//! OWNED BY THE EMULATOR AGENT. The public items declared here (and in `wire` / `client`) are a
//! CONTRACT shared with the Tauri app (`src-tauri`). Keep their names and signatures stable;
//! fill in the bodies and add whatever private modules you need.
//!
//! Layers:
//! - [`EmulatedStrap`]: synchronous, deterministic strap core driven by a caller-supplied clock.
//!   It speaks the real byte-level protocol from `unstrapped_core::protocol` (fragmented notifications,
//!   correct CRCs, real command handling incl. history range / chunk / ack).
//! - [`server`]: the TCP "service" (`cargo run -p unstrapped-emulator`) that wraps an `EmulatedStrap`
//!   and exposes it over the [`wire`] protocol, so the app can talk to it like a BLE peripheral.
//! - [`client`]: async client the Tauri app uses as its "simulated strap" transport.

pub mod client;
pub mod physio;
pub mod rng;
pub mod server;
pub mod strap;
pub mod wire;

pub use strap::{EmulatedStrap, StrapOptions};

pub use unstrapped_core::protocol::DeviceFamily;

/// Default address the emulator service listens on.
pub const DEFAULT_ADDR: &str = "127.0.0.1:47800";

/// Body / lifestyle the synthetic data is generated from.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SubjectProfile {
    pub age_years: u32,
    pub sex: String, // "male" | "female"
    pub weight_kg: f64,
    pub resting_hr_bpm: f64,
    pub hrv_rmssd_ms: f64,
    pub max_hr_bpm: f64,
    /// Typical bedtime / wake time as local minutes after midnight.
    pub bedtime_min: u32,
    pub wake_min: u32,
}

impl Default for SubjectProfile {
    fn default() -> Self {
        Self {
            age_years: 32,
            sex: "male".into(),
            weight_kg: 78.0,
            resting_hr_bpm: 54.0,
            hrv_rmssd_ms: 62.0,
            max_hr_bpm: 186.0,
            bedtime_min: 23 * 60,
            wake_min: 7 * 60,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EmulatorConfig {
    pub family: DeviceFamily,
    /// RNG seed — same seed + same clock ⇒ byte-identical output.
    pub seed: u64,
    /// How many days of history the strap holds when it boots (max 14, like the real strap).
    pub history_days: u32,
    /// Emulated wall clock at boot, Unix ms. `None` = real `SystemTime::now()`.
    pub start_time_ms: Option<i64>,
    /// Emulated-time speed multiplier for the server (1.0 = real time, 60.0 = 1 min per second).
    pub time_scale: f64,
    pub profile: SubjectProfile,
    /// Advertised device name, e.g. "WHOOP 4A1B2C3D".
    pub device_name: String,
}

impl Default for EmulatorConfig {
    fn default() -> Self {
        Self {
            family: DeviceFamily::Whoop4,
            seed: 42,
            history_days: 14,
            start_time_ms: None,
            time_scale: 1.0,
            profile: SubjectProfile::default(),
            device_name: "WHOOP 4EMU0001".into(),
        }
    }
}

/// A GATT characteristic the emulated strap exposes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CharacteristicInfo {
    pub uuid: String,
    pub notify: bool,
    pub write: bool,
}

/// One BLE notification (already fragmented to the emulated MTU, exactly like real hardware).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub characteristic: String,
    pub bytes: Vec<u8>,
}

/// The system's current UTC offset in minutes (best effort: `date +%z`, falling back to 0).
pub fn local_utc_offset_min() -> i32 {
    let out = std::process::Command::new("date").arg("+%z").output().ok();
    let s = out.and_then(|o| String::from_utf8(o.stdout).ok()).unwrap_or_default();
    let s = s.trim();
    if s.len() == 5 && (s.starts_with('+') || s.starts_with('-')) {
        let sign = if s.starts_with('-') { -1 } else { 1 };
        if let (Ok(h), Ok(m)) = (s[1..3].parse::<i32>(), s[3..5].parse::<i32>()) {
            return sign * (h * 60 + m);
        }
    }
    0
}
