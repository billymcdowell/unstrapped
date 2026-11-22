//! Standard Bluetooth Heart Rate Measurement (`0x2A37`), available on both strap generations and
//! usable without bonding. Live HR plus R-R intervals without the proprietary protocol.
//!
//! Layout: flags byte, then heart rate (u8, or u16 LE if flag bit 0), then optional Energy Expended
//! (u16, flag bit 3), then R-R intervals (u16 LE each, units of 1/1024 s, flag bit 4) until the end.
//! Sensor-contact bits: bit 2 = feature supported, bit 1 = contact detected.

use super::util::PacketError;

/// A decoded Heart Rate Measurement notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StdHeartRate {
    /// Heart rate in bpm.
    pub bpm: u16,
    /// Sensor contact: `Some(detected)` if the sensor supports the feature.
    pub contact: Option<bool>,
    /// Energy expended in kJ, if sent.
    pub energy_kj: Option<u16>,
    /// R-R intervals converted to milliseconds.
    pub rr_ms: Vec<u16>,
}

fn rr_to_ms(units: u16) -> u16 {
    ((units as u32 * 1000 + 512) / 1024).min(u16::MAX as u32) as u16
}

fn rr_from_ms(ms: u16) -> u16 {
    ((ms as u32 * 1024 + 500) / 1000).min(u16::MAX as u32) as u16
}

impl StdHeartRate {
    /// Parse a `0x2A37` value.
    pub fn decode(b: &[u8]) -> Result<Self, PacketError> {
        let trunc = |need: usize| PacketError::Truncated { what: "heart rate measurement", need, have: b.len() };
        let flags = *b.first().ok_or_else(|| trunc(1))?;
        let mut o = 1;
        let bpm = if flags & 1 != 0 {
            if b.len() < o + 2 {
                return Err(trunc(o + 2));
            }
            o += 2;
            u16::from_le_bytes([b[o - 2], b[o - 1]])
        } else {
            if b.len() < o + 1 {
                return Err(trunc(o + 1));
            }
            o += 1;
            b[o - 1] as u16
        };
        let contact = (flags & 0b100 != 0).then_some(flags & 0b010 != 0);
        let energy_kj = if flags & 0b1000 != 0 {
            if b.len() < o + 2 {
                return Err(trunc(o + 2));
            }
            o += 2;
            Some(u16::from_le_bytes([b[o - 2], b[o - 1]]))
        } else {
            None
        };
        let mut rr_ms = Vec::new();
        if flags & 0b1_0000 != 0 {
            while o + 2 <= b.len() {
                rr_ms.push(rr_to_ms(u16::from_le_bytes([b[o], b[o + 1]])));
                o += 2;
            }
        }
        Ok(StdHeartRate { bpm, contact, energy_kj, rr_ms })
    }

    /// Serialize (emulator side). Uses the 8-bit form when `bpm <= 255`.
    pub fn encode(&self) -> Vec<u8> {
        let mut flags = 0u8;
        if self.bpm > 255 {
            flags |= 1;
        }
        if let Some(c) = self.contact {
            flags |= 0b100 | if c { 0b010 } else { 0 };
        }
        if self.energy_kj.is_some() {
            flags |= 0b1000;
        }
        if !self.rr_ms.is_empty() {
            flags |= 0b1_0000;
        }
        let mut out = vec![flags];
        if self.bpm > 255 {
            out.extend_from_slice(&self.bpm.to_le_bytes());
        } else {
            out.push(self.bpm as u8);
        }
        if let Some(e) = self.energy_kj {
            out.extend_from_slice(&e.to_le_bytes());
        }
        for &ms in &self.rr_ms {
            out.extend_from_slice(&rr_from_ms(ms).to_le_bytes());
        }
        out
    }
}
