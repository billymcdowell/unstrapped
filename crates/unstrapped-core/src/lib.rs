//! unstrapped-core: pure-Rust WHOOP strap protocol + metrics. No Tauri, no Bluetooth, no I/O.
//!
//! - [`protocol`]: device families, GATT UUIDs, reassembly, CRCs, packet encode/decode, commands.
//! - [`metrics`]: HRV, resting HR, sleep staging, baselines, Recovery / Sleep / Strain / Stress scores.

pub mod metrics;
pub mod protocol;
