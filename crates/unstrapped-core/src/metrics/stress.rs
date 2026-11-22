//! Live stress 0-3: current HR and HRV against a 14-day baseline, ignoring movement.

use super::baselines::*;
use super::records::StressPoint;
use super::signals::*;
use super::types::*;
use super::util::*;
use serde::{Deserialize, Serialize};

/// One stress point per 5-minute window.
pub const STRESS_WINDOW_MIN: i64 = 5;
/// Windows whose mean motion exceeds this (g) are skipped, so exercise is not counted as stress.
pub const STRESS_MOVE_G: f64 = 0.05;
const MIN_HR_SAMPLES: usize = 10;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StressBand {
    Low,
    Medium,
    High,
}

/// Low < 1, medium 1-2, high >= 2.
pub fn stress_band(score: f64) -> StressBand {
    if score < 1.0 {
        StressBand::Low
    } else if score < 2.0 {
        StressBand::Medium
    } else {
        StressBand::High
    }
}

/// Reference (calm) HR / HRV and their spreads.
#[derive(Debug, Clone, Copy)]
pub struct StressRef {
    pub hr_mu: f64,
    pub hr_sigma: f64,
    pub hrv_mu: Option<f64>,
    /// Relative spread of HRV (sigma / mu).
    pub hrv_rel_sigma: Option<f64>,
}

impl StressRef {
    pub fn from_baselines(hr: &BaselineStat, hrv: Option<&BaselineStat>) -> Self {
        StressRef {
            hr_mu: hr.mean,
            hr_sigma: hr.sigma,
            hrv_mu: hrv.map(|b| b.mean),
            hrv_rel_sigma: hrv.map(|b| (b.sigma / b.mean).max(0.08)),
        }
    }
}

/// Stress 0-3 for one window. `x = mean(z_hr, z_hrv)` with `z_hr = (hr - mu)/sigma` and
/// `z_hrv = (ln mu_hrv - ln rmssd)/rel_sigma` (higher HR and lower HRV = more stress), then
/// `score = 3 x logistic(1.2 (x - 0.8))`: a calm baseline window is ~0.8, +1 sd ~1.6, +2 sd ~2.3.
pub fn stress_score(hr: f64, rmssd: Option<f64>, r: &StressRef) -> f64 {
    let mut zs = vec![(hr - r.hr_mu) / r.hr_sigma];
    if let (Some(v), Some(mu), Some(rs)) = (rmssd.filter(|v| *v > 0.0), r.hrv_mu, r.hrv_rel_sigma) {
        zs.push((mu.ln() - v.ln()) / rs);
    }
    let x = zs.iter().sum::<f64>() / zs.len() as f64;
    3.0 * logistic(1.2 * (x - 0.8))
}

#[derive(Debug, Clone, Copy)]
struct Win {
    ts: i64,
    hr: f64,
    rmssd: Option<f64>,
    asleep: bool,
}

fn own_ref(w: &[&Win]) -> Option<StressRef> {
    if w.len() < 3 {
        return None;
    }
    let hrs: Vec<f64> = w.iter().map(|x| x.hr).collect();
    let hrv: Vec<f64> = w.iter().filter_map(|x| x.rmssd).collect();
    let hr_mu = median(&hrs)?;
    let hr_sigma = std_dev(&hrs).unwrap_or(3.0).max(3.0);
    let (hrv_mu, rel) = if hrv.len() >= 3 {
        let mu = median(&hrv)?;
        let lg: Vec<f64> = hrv.iter().map(|v| v.ln()).collect();
        (Some(mu), Some(std_dev(&lg).unwrap_or(0.15).max(0.08)))
    } else {
        (None, None)
    };
    Some(StressRef { hr_mu, hr_sigma, hrv_mu, hrv_rel_sigma: rel })
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct StressSummary {
    pub current: Option<f64>,
    pub current_band: Option<StressBand>,
    pub low_min: i64,
    pub medium_min: i64,
    pub high_min: i64,
    /// Share (0..1) of sleep-window points in the high band; `None` with < 30 min of sleep points.
    pub sleep_high_share: Option<f64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct StressSeries {
    pub points: Vec<StressPoint>,
    pub summary: StressSummary,
    /// Median HR / RMSSD over still, awake windows (feeds the next day's 14-day stress baseline).
    pub ref_hr: Option<f64>,
    pub ref_hrv: Option<f64>,
    /// True when no 14-day baseline existed and the day's own distribution was used instead.
    pub provisional: bool,
}

/// Stress points for `[start,end)`. Awake windows use the `awake_base` baselines (daytime stress HR/HRV);
/// windows inside `sleeps` use the sleeping baselines `sleep_base`. Missing baselines fall back to the
/// day's own still-window distribution (flagged `provisional`). Moving windows are skipped.
#[allow(clippy::too_many_arguments)]
pub fn stress_series(
    hr: &[HrSample],
    rr: &[RrInterval],
    motion: &[MotionSample],
    start: i64,
    end: i64,
    sleeps: &[(i64, i64)],
    awake_base: (Option<BaselineStat>, Option<BaselineStat>),
    sleep_base: (Option<BaselineStat>, Option<BaselineStat>),
) -> StressSeries {
    let wl = STRESS_WINDOW_MIN * MS_PER_MIN;
    let mut wins: Vec<Win> = Vec::new();
    let mut t = start;
    while t + wl <= end {
        let h = hr_in(hr, t, t + wl);
        if h.len() >= MIN_HR_SAMPLES {
            let m = motion_in(motion, t, t + wl);
            let moving = !m.is_empty() && m.iter().map(|s| s.magnitude_g).sum::<f64>() / m.len() as f64 > STRESS_MOVE_G;
            if !moving {
                let mid = t + wl / 2;
                wins.push(Win {
                    ts: t + wl,
                    hr: h.iter().map(|s| s.bpm).sum::<f64>() / h.len() as f64,
                    rmssd: hrv_rmssd(rr_in(rr, t, t + wl), 30).map(|r| r.rmssd_ms),
                    asleep: sleeps.iter().any(|(a, b)| mid >= *a && mid < *b),
                });
            }
        }
        t += wl;
    }
    let awake: Vec<&Win> = wins.iter().filter(|w| !w.asleep).collect();
    let asleep: Vec<&Win> = wins.iter().filter(|w| w.asleep).collect();
    let mut provisional = false;
    let mut resolve = |base: (Option<BaselineStat>, Option<BaselineStat>), own: &[&Win]| -> Option<StressRef> {
        match base.0 {
            Some(h) => Some(StressRef::from_baselines(&h, base.1.as_ref())),
            None => {
                provisional |= !own.is_empty();
                own_ref(own)
            }
        }
    };
    let ref_awake = resolve(awake_base, &awake);
    let ref_sleep = resolve(sleep_base, &asleep);
    let points: Vec<StressPoint> = wins
        .iter()
        .filter_map(|w| {
            let r = if w.asleep { ref_sleep } else { ref_awake }?;
            Some(StressPoint { ts_ms: w.ts, score: clamp(stress_score(w.hr, w.rmssd, &r), 0.0, 3.0) })
        })
        .collect();
    let summary = summarize_stress(&points, sleeps);
    let ref_hr = median(&awake.iter().map(|w| w.hr).collect::<Vec<_>>());
    let ref_hrv = median(&awake.iter().filter_map(|w| w.rmssd).collect::<Vec<_>>());
    StressSeries { points, summary, ref_hr, ref_hrv, provisional }
}

/// Time in band (each point = 5 min), the current value, and the Sleep Stress share.
pub fn summarize_stress(points: &[StressPoint], sleeps: &[(i64, i64)]) -> StressSummary {
    let (mut lo, mut me, mut hi) = (0, 0, 0);
    for p in points {
        match stress_band(p.score) {
            StressBand::Low => lo += STRESS_WINDOW_MIN,
            StressBand::Medium => me += STRESS_WINDOW_MIN,
            StressBand::High => hi += STRESS_WINDOW_MIN,
        }
    }
    let in_sleep: Vec<&StressPoint> = points.iter().filter(|p| sleeps.iter().any(|(a, b)| p.ts_ms > *a && p.ts_ms <= *b)).collect();
    let sleep_high_share = if in_sleep.len() >= 6 {
        Some(in_sleep.iter().filter(|p| stress_band(p.score) == StressBand::High).count() as f64 / in_sleep.len() as f64)
    } else {
        None
    };
    let cur = points.last().map(|p| p.score);
    StressSummary { current: cur, current_band: cur.map(stress_band), low_min: lo, medium_min: me, high_min: hi, sleep_high_share }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r() -> StressRef {
        // HR 70 +- 5; HRV 50 ms, relative sigma 0.2
        StressRef { hr_mu: 70.0, hr_sigma: 5.0, hrv_mu: Some(50.0), hrv_rel_sigma: Some(0.2) }
    }

    #[test]
    fn score_hand_worked() {
        // at baseline: x = 0 -> 3*logistic(-0.96) = 3*0.2769 = 0.831
        assert!((stress_score(70.0, Some(50.0), &r()) - 0.831).abs() < 0.005);
        // HR 80 (z=+2) and HRV 50*exp(-0.4)=33.5 (z = 0.4/0.2 = +2): x=2 -> 3*logistic(1.44) = 3*0.8082 = 2.425
        let v = 50.0 * (-0.4f64).exp();
        assert!((stress_score(80.0, Some(v), &r()) - 2.425).abs() < 0.005);
        // calmer than baseline: HR 62 (z=-1.6), HRV 70 (z = (ln50-ln70)/0.2 = -1.68) -> x=-1.64 -> 3*logistic(-2.93)= 0.15
        assert!(stress_score(62.0, Some(70.0), &r()) < 0.3);
        // HR only
        let hr_only = StressRef { hrv_mu: None, hrv_rel_sigma: None, ..r() };
        assert!((stress_score(75.0, None, &hr_only) - 3.0 * logistic(1.2 * (1.0 - 0.8))).abs() < 1e-9);
    }

    #[test]
    fn bands() {
        assert_eq!(stress_band(0.99), StressBand::Low);
        assert_eq!(stress_band(1.0), StressBand::Medium);
        assert_eq!(stress_band(2.0), StressBand::High);
    }

    #[test]
    fn summary_counts_time_in_band_and_sleep_share() {
        let mk = |ts, s| StressPoint { ts_ms: ts, score: s };
        let pts: Vec<_> = (1..=10).map(|i| mk(i * 300_000, if i <= 6 { 0.5 } else if i <= 8 { 1.5 } else { 2.5 })).collect();
        let s = summarize_stress(&pts, &[(0, 3_000_000)]);
        assert_eq!((s.low_min, s.medium_min, s.high_min), (30, 10, 10));
        assert_eq!(s.current, Some(2.5));
        assert_eq!(s.current_band, Some(StressBand::High));
        // 10 points in sleep, 2 high -> 0.2
        assert!((s.sleep_high_share.unwrap() - 0.2).abs() < 1e-9);
        assert_eq!(summarize_stress(&pts[..3], &[(0, 3_000_000)]).sleep_high_share, None);
    }

    #[test]
    fn movement_windows_are_ignored_and_spike_registers() {
        // 60 min: 0-20 calm HR 70, 20-30 running HR 150 (motion), 30-40 calm, 40-50 stressed (HR 85 still), 50-60 calm
        let mut hr = Vec::new();
        let mut mo = Vec::new();
        for s in 0..3600i64 {
            let m = s / 60;
            let (b, g) = match m {
                20..=29 => (150.0, 0.4),
                40..=49 => (85.0, 0.005),
                _ => (70.0, 0.005),
            };
            hr.push(HrSample { ts_ms: s * 1000, bpm: b });
            mo.push(MotionSample { ts_ms: s * 1000, magnitude_g: g });
        }
        let base = BaselineStat { mean: 70.0, sigma: 5.0, lo: 65.0, hi: 75.0, nights: 14 };
        let s = stress_series(&hr, &[], &mo, 0, 3_600_000, &[], (Some(base), None), (None, None));
        // 12 windows minus 2 moving = 10 points
        assert_eq!(s.points.len(), 10, "{:?}", s.points);
        assert!(!s.provisional);
        let at = |ts| s.points.iter().find(|p| p.ts_ms == ts).unwrap().score;
        assert!(at(2 * 300_000) < 1.0); // calm: ~0.83
        assert!(at(9 * 300_000) > 2.0); // HR 85 = +3 sd -> 3*logistic(1.2*2.2) = 2.8
        // windows covering minutes 20-30 (ts 25 and 30 min) are absent
        assert!(s.points.iter().all(|p| p.ts_ms != 5 * 300_000 && p.ts_ms != 6 * 300_000));
    }

    #[test]
    fn falls_back_to_own_distribution_and_says_so() {
        let hr: Vec<_> = (0..3600).map(|s| HrSample { ts_ms: s * 1000, bpm: 70.0 + (s / 300 % 3) as f64 * 3.0 }).collect();
        let s = stress_series(&hr, &[], &[], 0, 3_600_000, &[], (None, None), (None, None));
        assert!(s.provisional);
        assert_eq!(s.points.len(), 12);
        assert!(s.ref_hr.is_some());
    }
}
