//! Personal baselines: winsorised EWMA mean + spread per metric (~14-night half-life).

use super::records::*;
use serde::{Deserialize, Serialize};

pub const CENTER_HALF_LIFE: f64 = 14.0;
pub const SPREAD_HALF_LIFE: f64 = 21.0;
/// Fewer valid nights than this: score is "Calibrating".
pub const MIN_NIGHTS: u32 = 4;
/// At or above this many nights the baseline is "solid".
pub const SOLID_NIGHTS: u32 = 14;
const WINSOR_K: f64 = 3.0;
const HARD_OUTLIER_K: f64 = 5.0;
/// mean-absolute-deviation -> Gaussian sigma.
const MAD_TO_SIGMA: f64 = 1.253;

#[derive(Clone, Copy, Debug)]
pub struct MetricCfg {
    pub min: f64,
    pub max: f64,
    pub floor_spread: f64,
}

pub const CFG_HRV: MetricCfg = MetricCfg { min: 5.0, max: 250.0, floor_spread: 4.0 };
pub const CFG_RHR: MetricCfg = MetricCfg { min: 30.0, max: 120.0, floor_spread: 1.5 };
pub const CFG_RESP: MetricCfg = MetricCfg { min: 4.0, max: 40.0, floor_spread: 0.4 };
pub const CFG_TEMP: MetricCfg = MetricCfg { min: 20.0, max: 42.0, floor_spread: 0.2 };
pub const CFG_SPO2: MetricCfg = MetricCfg { min: 70.0, max: 100.0, floor_spread: 0.5 };
pub const CFG_STRESS_HR: MetricCfg = MetricCfg { min: 30.0, max: 160.0, floor_spread: 3.0 };
pub const CFG_STRESS_HRV: MetricCfg = MetricCfg { min: 5.0, max: 250.0, floor_spread: 5.0 };

/// A baseline: centre, Gaussian-equivalent spread (`sigma`), and the normal range `mean +- sigma`.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct BaselineStat {
    pub mean: f64,
    pub sigma: f64,
    pub lo: f64,
    pub hi: f64,
    pub nights: u32,
}

impl BaselineStat {
    pub fn z(&self, v: f64) -> f64 {
        (v - self.mean) / self.sigma
    }
}

/// Fold a chronological series of nightly values into a baseline.
///
/// The first 4 valid values seed the centre (plain mean) and spread (mean absolute deviation, floored);
/// later values update both with weights `1 - 0.5^(1/half_life)`, winsorised at +-3 spreads, and
/// values more than 5 spreads away are ignored as outliers. Returns `None` with no valid values.
pub fn fold_baseline(values: &[f64], cfg: MetricCfg) -> Option<BaselineStat> {
    let valid: Vec<f64> = values.iter().copied().filter(|v| v.is_finite() && *v >= cfg.min && *v <= cfg.max).collect();
    if valid.is_empty() {
        return None;
    }
    let seed_n = valid.len().min(MIN_NIGHTS as usize);
    let mut mean_v = valid[..seed_n].iter().sum::<f64>() / seed_n as f64;
    let mad = valid[..seed_n].iter().map(|v| (v - mean_v).abs()).sum::<f64>() / seed_n as f64;
    let mut spread = mad.max(cfg.floor_spread);
    let mut nights = seed_n as u32;
    let lc = 1.0 - 0.5f64.powf(1.0 / CENTER_HALF_LIFE);
    let ls = 1.0 - 0.5f64.powf(1.0 / SPREAD_HALF_LIFE);
    for &v in &valid[seed_n..] {
        if (v - mean_v).abs() > HARD_OUTLIER_K * spread {
            continue;
        }
        let clamped = v.clamp(mean_v - WINSOR_K * spread, mean_v + WINSOR_K * spread);
        mean_v = lc * clamped + (1.0 - lc) * mean_v;
        spread = (ls * (v - mean_v).abs() + (1.0 - ls) * spread).max(cfg.floor_spread);
        nights += 1;
    }
    let sigma = MAD_TO_SIGMA * spread;
    Some(BaselineStat { mean: mean_v, sigma, lo: mean_v - sigma, hi: mean_v + sigma, nights })
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct BaselineSet {
    pub hrv: Option<BaselineStat>,
    pub resting_hr: Option<BaselineStat>,
    pub resp_rate: Option<BaselineStat>,
    /// Baseline of the absolute sleeping skin temperature (deg C).
    pub skin_temp: Option<BaselineStat>,
    pub spo2: Option<BaselineStat>,
    pub stress_hr: Option<BaselineStat>,
    pub stress_hrv: Option<BaselineStat>,
}

/// Calibration state from a count of valid nights.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Calibration {
    Calibrating { nights_remaining: u32 },
    /// 4..13 nights: usable but uncertain.
    Provisional { nights: u32 },
    Solid { nights: u32 },
}

pub fn calibration_state(nights: u32) -> Calibration {
    if nights < MIN_NIGHTS {
        Calibration::Calibrating { nights_remaining: MIN_NIGHTS - nights }
    } else if nights < SOLID_NIGHTS {
        Calibration::Provisional { nights }
    } else {
        Calibration::Solid { nights }
    }
}

/// Maximum history (days) folded into a baseline.
const HISTORY_DAYS: usize = 90;

/// Build the baselines as they stood on the morning of `day`: from history days strictly before it.
pub fn baselines_from_history(history: &[DailySummary], day: &str) -> BaselineSet {
    let mut h: Vec<&DailySummary> = history.iter().filter(|d| d.day.as_str() < day).collect();
    h.sort_by(|a, b| a.day.cmp(&b.day));
    let start = h.len().saturating_sub(HISTORY_DAYS);
    let h = &h[start..];
    let series = |f: &dyn Fn(&DailySummary) -> Option<f64>, cfg: MetricCfg| {
        let v: Vec<f64> = h.iter().filter_map(|d| f(d)).collect();
        fold_baseline(&v, cfg)
    };
    BaselineSet {
        hrv: series(&|d| d.hrv_ms, CFG_HRV),
        resting_hr: series(&|d| d.resting_hr, CFG_RHR),
        resp_rate: series(&|d| d.resp_rate, CFG_RESP),
        skin_temp: series(&|d| d.skin_temp_c, CFG_TEMP),
        spo2: series(&|d| d.spo2, CFG_SPO2),
        stress_hr: series(&|d| d.stress_ref_hr, CFG_STRESS_HR),
        stress_hrv: series(&|d| d.stress_ref_hrv, CFG_STRESS_HRV),
    }
}

/// Rows for the `baselines` table (metric keys: hrv, resting_hr, resp, skin_temp, spo2, stress_hr, stress_hrv).
pub fn baseline_rows(day: &str, set: &BaselineSet) -> Vec<BaselineRow> {
    let mut out = Vec::new();
    for (name, b) in [
        ("hrv", &set.hrv),
        ("resting_hr", &set.resting_hr),
        ("resp", &set.resp_rate),
        ("skin_temp", &set.skin_temp),
        ("spo2", &set.spo2),
        ("stress_hr", &set.stress_hr),
        ("stress_hrv", &set.stress_hrv),
    ] {
        if let Some(b) = b {
            out.push(BaselineRow { day: day.to_string(), metric: name.to_string(), mean: b.mean, lo: b.lo, hi: b.hi });
        }
    }
    out
}

/// Number of prior nights with an HRV value (the calibration counter for Recovery).
pub fn valid_nights(history: &[DailySummary], day: &str) -> u32 {
    history.iter().filter(|d| d.day.as_str() < day && d.hrv_ms.is_some() && d.resting_hr.is_some()).count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_from_first_four_nights() {
        // 60,64,56,60: mean 60, abs devs 0,4,4,0 -> MAD 2 -> floor 4 (hrv) -> spread 4 -> sigma 5.012
        let b = fold_baseline(&[60.0, 64.0, 56.0, 60.0], CFG_HRV).unwrap();
        assert!((b.mean - 60.0).abs() < 1e-9);
        assert!((b.sigma - 4.0 * 1.253).abs() < 1e-9);
        assert!((b.lo - (60.0 - b.sigma)).abs() < 1e-9);
        assert_eq!(b.nights, 4);
    }

    #[test]
    fn ewma_weights_recent_nights() {
        // 4 seed nights at 60, then ten nights at 70: centre moves toward 70 but not all the way.
        let mut v = vec![60.0; 4];
        v.extend(vec![70.0; 10]);
        let b = fold_baseline(&v, CFG_HRV).unwrap();
        // lambda = 1 - 0.5^(1/14) = 0.04830; 10 winsorised updates: 60 + 10*(1-(1-l)^10) -> 60 + 10*0.4095 = ~64.1
        // (winsor clamp: floor spread 4 -> +-12 so 70 is inside, no clamping)
        assert!(b.mean > 63.5 && b.mean < 64.7, "mean {}", b.mean);
    }

    #[test]
    fn hard_outlier_is_ignored() {
        let mut v = vec![60.0, 62.0, 58.0, 60.0];
        v.push(200.0); // > 5 spreads away
        let b = fold_baseline(&v, CFG_HRV).unwrap();
        assert_eq!(b.nights, 4);
        assert!((b.mean - 60.0).abs() < 1e-9);
    }

    #[test]
    fn invalid_values_dropped_and_empty_is_none() {
        assert!(fold_baseline(&[], CFG_HRV).is_none());
        assert!(fold_baseline(&[0.0, 1.0, 999.0], CFG_HRV).is_none());
    }

    #[test]
    fn calibration_thresholds() {
        assert_eq!(calibration_state(0), Calibration::Calibrating { nights_remaining: 4 });
        assert_eq!(calibration_state(3), Calibration::Calibrating { nights_remaining: 1 });
        assert_eq!(calibration_state(4), Calibration::Provisional { nights: 4 });
        assert_eq!(calibration_state(14), Calibration::Solid { nights: 14 });
    }

    #[test]
    fn history_only_uses_days_before() {
        let mk = |d: &str, h: f64| DailySummary { day: d.into(), hrv_ms: Some(h), resting_hr: Some(50.0), ..Default::default() };
        let hist = vec![mk("2026-01-01", 60.0), mk("2026-01-02", 60.0), mk("2026-01-03", 60.0), mk("2026-01-04", 60.0), mk("2026-01-05", 100.0)];
        let b = baselines_from_history(&hist, "2026-01-05");
        assert!((b.hrv.unwrap().mean - 60.0).abs() < 1e-9);
        assert_eq!(valid_nights(&hist, "2026-01-05"), 4);
        assert_eq!(baseline_rows("2026-01-05", &b).len(), 2); // hrv + resting_hr only
    }
}
