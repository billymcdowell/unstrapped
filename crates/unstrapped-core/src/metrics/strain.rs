//! Strain (0-21): zone-weighted load on a log scale; bands and the Strain Target.

use super::hr::*;
use super::types::*;
use serde::{Deserialize, Serialize};

/// ln(7201): a full day (1440 min) in zone 5 (weight 5) is a load of 7200 and maps to exactly 21.
pub const STRAIN_DENOM: f64 = 7201.0;
pub const MAX_STRAIN: f64 = 21.0;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryBand {
    Red,
    Yellow,
    Green,
}

impl RecoveryBand {
    pub fn as_str(self) -> &'static str {
        match self {
            RecoveryBand::Red => "red",
            RecoveryBand::Yellow => "yellow",
            RecoveryBand::Green => "green",
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StrainBand {
    Light,
    Moderate,
    High,
    AllOut,
}

/// Load = sum over HR samples of (zone number 1..5) x minutes. Below zone 1 contributes nothing.
pub fn training_load(hr: &[HrSample], zones: &HrZones, start: i64, end: i64) -> f64 {
    let m = time_in_zones(hr, zones, start, end);
    m.iter().enumerate().map(|(i, min)| (i + 1) as f64 * min).sum()
}

/// `21 x ln(load + 1) / ln(7201)`, capped at 21.
pub fn strain_from_load(load: f64) -> f64 {
    (MAX_STRAIN * (load.max(0.0) + 1.0).ln() / STRAIN_DENOM.ln()).min(MAX_STRAIN)
}

pub fn strain_for_window(hr: &[HrSample], zones: &HrZones, start: i64, end: i64) -> f64 {
    strain_from_load(training_load(hr, zones, start, end))
}

/// Light 0-9.99, Moderate 10-13.99, High 14-17.99, All out 18-21.
pub fn strain_band(strain: f64) -> StrainBand {
    if strain < 10.0 {
        StrainBand::Light
    } else if strain < 14.0 {
        StrainBand::Moderate
    } else if strain < 18.0 {
        StrainBand::High
    } else {
        StrainBand::AllOut
    }
}

/// Recommended strain range for the day from the recovery band.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct StrainTarget {
    pub lo: f64,
    pub hi: f64,
}

pub fn strain_target(band: RecoveryBand) -> StrainTarget {
    match band {
        RecoveryBand::Green => StrainTarget { lo: 14.0, hi: 18.0 },
        RecoveryBand::Yellow => StrainTarget { lo: 10.0, hi: 14.0 },
        RecoveryBand::Red => StrainTarget { lo: 0.0, hi: 10.0 },
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct StrainDetail {
    pub day_strain: f64,
    pub band: StrainBand,
    pub load: f64,
    pub target: Option<StrainTarget>,
    pub zone_minutes: [f64; 5],
    pub max_hr: f64,
    pub resting_hr_used: f64,
    pub zones: HrZones,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strain_formula_hand_worked() {
        // load 0 -> 0
        assert!(strain_from_load(0.0).abs() < 1e-12);
        // load 120 (60 min in zone 2): 21*ln(121)/ln(7201) = 21*4.79579/8.88183 = 11.339
        assert!((strain_from_load(120.0) - 11.339).abs() < 0.01);
        // load 7200 -> exactly 21
        assert!((strain_from_load(7200.0) - 21.0).abs() < 1e-9);
        // never above 21
        assert_eq!(strain_from_load(1e9), 21.0);
        // monotone with diminishing returns
        assert!(strain_from_load(200.0) - strain_from_load(100.0) < strain_from_load(100.0) - strain_from_load(0.0));
    }

    #[test]
    fn load_from_zone_time() {
        let z = hr_zones(190.0, 50.0);
        // 20 min in zone 3 (150 bpm) + 10 min in zone 5 (180): 20*3 + 10*5 = 110
        let mut hr = Vec::new();
        for i in 0..1200 {
            hr.push(HrSample { ts_ms: i * 1000, bpm: 150.0 });
        }
        for i in 1200..1800 {
            hr.push(HrSample { ts_ms: i * 1000, bpm: 180.0 });
        }
        let l = training_load(&hr, &z, 0, 1_800_000);
        assert!((l - 110.0).abs() < 0.5, "{l}");
        // strain = 21*ln(111)/ln(7201) = 21*4.7095/8.8818 = 11.135
        assert!((strain_for_window(&hr, &z, 0, 1_800_000) - 11.135).abs() < 0.05);
    }

    #[test]
    fn bands_and_targets() {
        assert_eq!(strain_band(9.9), StrainBand::Light);
        assert_eq!(strain_band(10.0), StrainBand::Moderate);
        assert_eq!(strain_band(13.9), StrainBand::Moderate);
        assert_eq!(strain_band(14.0), StrainBand::High);
        assert_eq!(strain_band(18.0), StrainBand::AllOut);
        assert_eq!(strain_target(RecoveryBand::Green), StrainTarget { lo: 14.0, hi: 18.0 });
        assert_eq!(strain_target(RecoveryBand::Yellow), StrainTarget { lo: 10.0, hi: 14.0 });
        assert_eq!(strain_target(RecoveryBand::Red), StrainTarget { lo: 0.0, hi: 10.0 });
    }
}
