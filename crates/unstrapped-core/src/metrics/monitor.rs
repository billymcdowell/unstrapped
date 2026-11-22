//! Health Monitor: each overnight metric against its normal range, plus the early-warning flag.

use super::baselines::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MonitorStatus {
    /// Within 1 spread of the baseline (green check).
    InRange,
    /// 1-2 spreads away (orange).
    SlightlyOutside,
    /// More than 2 spreads away (red).
    WellOutside,
    /// No baseline yet (calibrating) or no reading.
    NoData,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MonitorMetric {
    /// `resp_rate`, `spo2`, `resting_hr`, `hrv`, `skin_temp`.
    pub metric: String,
    pub value: Option<f64>,
    pub baseline_mean: Option<f64>,
    pub range_lo: Option<f64>,
    pub range_hi: Option<f64>,
    /// Signed spreads from baseline (positive = above).
    pub z: Option<f64>,
    pub status: MonitorStatus,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct HealthMonitor {
    pub metrics: Vec<MonitorMetric>,
    pub in_range_count: usize,
    pub total_with_data: usize,
    /// Respiratory rate up, resting HR up and HRV down together (each beyond 1 spread).
    pub early_warning: bool,
    pub early_warning_signals: Vec<String>,
    pub nights_of_baseline: u32,
}

pub fn status_for_z(z: f64) -> MonitorStatus {
    let a = z.abs();
    if a <= 1.0 {
        MonitorStatus::InRange
    } else if a <= 2.0 {
        MonitorStatus::SlightlyOutside
    } else {
        MonitorStatus::WellOutside
    }
}

fn metric(name: &str, value: Option<f64>, base: Option<&BaselineStat>, nights_ok: bool) -> MonitorMetric {
    match (value, base) {
        (Some(v), Some(b)) if nights_ok => {
            let z = b.z(v);
            MonitorMetric { metric: name.into(), value, baseline_mean: Some(b.mean), range_lo: Some(b.lo), range_hi: Some(b.hi), z: Some(z), status: status_for_z(z) }
        }
        (v, b) => MonitorMetric {
            metric: name.into(),
            value: v,
            baseline_mean: b.map(|b| b.mean),
            range_lo: b.map(|b| b.lo),
            range_hi: b.map(|b| b.hi),
            z: None,
            status: MonitorStatus::NoData,
        },
    }
}

/// Status per metric from the baselines. `skin_temp_c` is the absolute sleeping temperature (compared with
/// the temperature baseline). Metrics need >= 4 baseline nights, else `NoData`.
pub fn health_monitor(
    resp: Option<f64>,
    spo2: Option<f64>,
    rhr: Option<f64>,
    hrv: Option<f64>,
    skin_temp_c: Option<f64>,
    b: &BaselineSet,
    nights: u32,
) -> HealthMonitor {
    let ok = nights >= MIN_NIGHTS;
    let metrics = vec![
        metric("resp_rate", resp, b.resp_rate.as_ref(), ok),
        metric("spo2", spo2, b.spo2.as_ref(), ok),
        metric("resting_hr", rhr, b.resting_hr.as_ref(), ok),
        metric("hrv", hrv, b.hrv.as_ref(), ok),
        metric("skin_temp", skin_temp_c, b.skin_temp.as_ref(), ok),
    ];
    let with: Vec<&MonitorMetric> = metrics.iter().filter(|m| m.status != MonitorStatus::NoData).collect();
    let in_range = with.iter().filter(|m| m.status == MonitorStatus::InRange).count();
    let z = |n: &str| metrics.iter().find(|m| m.metric == n).and_then(|m| m.z);
    let mut sig = Vec::new();
    if z("resp_rate").is_some_and(|z| z > 1.0) {
        sig.push("resp_rate_up".to_string());
    }
    if z("resting_hr").is_some_and(|z| z > 1.0) {
        sig.push("resting_hr_up".to_string());
    }
    if z("hrv").is_some_and(|z| z < -1.0) {
        sig.push("hrv_down".to_string());
    }
    HealthMonitor {
        total_with_data: with.len(),
        in_range_count: in_range,
        early_warning: sig.len() == 3,
        early_warning_signals: sig,
        metrics,
        nights_of_baseline: nights,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bs(mean: f64, sigma: f64) -> BaselineStat {
        BaselineStat { mean, sigma, lo: mean - sigma, hi: mean + sigma, nights: 14 }
    }
    fn set() -> BaselineSet {
        BaselineSet { hrv: Some(bs(60.0, 5.0)), resting_hr: Some(bs(52.0, 2.0)), resp_rate: Some(bs(15.0, 0.5)), skin_temp: Some(bs(33.0, 0.3)), spo2: Some(bs(97.0, 0.5)), ..Default::default() }
    }

    #[test]
    fn status_thresholds() {
        assert_eq!(status_for_z(0.99), MonitorStatus::InRange);
        assert_eq!(status_for_z(-1.5), MonitorStatus::SlightlyOutside);
        assert_eq!(status_for_z(2.01), MonitorStatus::WellOutside);
    }

    #[test]
    fn all_in_range_no_warning() {
        let m = health_monitor(Some(15.2), Some(97.0), Some(52.5), Some(58.0), Some(33.1), &set(), 20);
        assert_eq!(m.in_range_count, 5);
        assert_eq!(m.total_with_data, 5);
        assert!(!m.early_warning);
    }

    #[test]
    fn early_warning_when_three_move_together() {
        // resp 16 (z=+2), RHR 55 (z=+1.5), HRV 48 (z=-2.4)
        let m = health_monitor(Some(16.0), None, Some(55.0), Some(48.0), None, &set(), 20);
        assert!(m.early_warning);
        assert_eq!(m.early_warning_signals.len(), 3);
        let resp = m.metrics.iter().find(|x| x.metric == "resp_rate").unwrap();
        assert_eq!(resp.status, MonitorStatus::SlightlyOutside); // z = 2.0 exactly -> slightly (<= 2)
        let hrv = m.metrics.iter().find(|x| x.metric == "hrv").unwrap();
        assert_eq!(hrv.status, MonitorStatus::WellOutside);
        // two of three is not an early warning
        let m2 = health_monitor(Some(16.0), None, Some(52.0), Some(48.0), None, &set(), 20);
        assert!(!m2.early_warning);
    }

    #[test]
    fn calibrating_gives_no_status() {
        let m = health_monitor(Some(16.0), None, Some(55.0), Some(48.0), None, &set(), 2);
        assert!(m.metrics.iter().all(|x| x.status == MonitorStatus::NoData));
        assert!(!m.early_warning);
    }
}
