//! Max HR, heart-rate zones, time in zones, calories, steps.

use super::signals::hr_in;
use super::types::*;
use super::util::*;
use serde::{Deserialize, Serialize};

/// Longest gap (ms) a single HR sample is credited for. Slower streams are under-counted, never extrapolated.
pub const MAX_SAMPLE_DT_MS: i64 = 15_000;

/// Tanaka (2001): 208 - 0.7 x age.
pub fn tanaka_max_hr(age: f64) -> f64 {
    208.0 - 0.7 * age
}

/// Max HR: the user's override if set; otherwise the larger of Tanaka and the observed 99.5th
/// percentile (only trusted with >= 600 samples).
pub fn max_hr(profile: &UserProfile, observed: &[HrSample]) -> f64 {
    if let Some(m) = profile.max_hr_override {
        return m;
    }
    let tanaka = tanaka_max_hr(profile.age);
    if observed.len() >= 600 {
        let v: Vec<f64> = observed.iter().map(|s| s.bpm).filter(|b| (30.0..=230.0).contains(b)).collect();
        if let Some(p) = percentile(&v, 99.5) {
            return p.max(tanaka);
        }
    }
    tanaka
}

/// Zones 1-5 start at 50/60/70/80/90 % of heart-rate reserve (Karvonen).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct HrZones {
    pub max_hr: f64,
    pub resting_hr: f64,
    /// Lower bpm bound of zones 1..5.
    pub lower_bpm: [f64; 5],
}

pub fn hr_zones(max_hr: f64, resting_hr: f64) -> HrZones {
    let reserve = (max_hr - resting_hr).max(1.0);
    let mut lower = [0.0; 5];
    for (i, l) in lower.iter_mut().enumerate() {
        *l = resting_hr + reserve * (0.5 + 0.1 * i as f64);
    }
    HrZones { max_hr, resting_hr, lower_bpm: lower }
}

impl HrZones {
    /// Zone 0 (below zone 1) to 5.
    pub fn zone(&self, bpm: f64) -> usize {
        self.lower_bpm.iter().rposition(|l| bpm >= *l).map(|i| i + 1).unwrap_or(0)
    }
    /// Percent of heart-rate reserve, clamped 0-100.
    pub fn pct_hrr(&self, bpm: f64) -> f64 {
        clamp((bpm - self.resting_hr) / (self.max_hr - self.resting_hr).max(1.0) * 100.0, 0.0, 100.0)
    }
}

/// Per-sample credited duration in seconds (gap to the next sample, capped at 15 s; the last sample
/// gets the median spacing).
pub fn sample_durations(hr: &[HrSample]) -> Vec<f64> {
    let mut d: Vec<f64> = hr.windows(2).map(|w| ((w[1].ts_ms - w[0].ts_ms).clamp(0, MAX_SAMPLE_DT_MS)) as f64 / 1000.0).collect();
    let last = median(&d).unwrap_or(1.0);
    d.push(last);
    d
}

/// Minutes spent in zones 1..5 (index 0 = zone 1) within `[start,end)`.
pub fn time_in_zones(hr: &[HrSample], zones: &HrZones, start: i64, end: i64) -> [f64; 5] {
    let s = hr_in(hr, start, end);
    let d = sample_durations(s);
    let mut m = [0.0; 5];
    for (x, dt) in s.iter().zip(d) {
        let z = zones.zone(x.bpm);
        if z >= 1 {
            m[z - 1] += dt / 60.0;
        }
    }
    m
}

/// Basal metabolic rate, kcal/day (Mifflin-St Jeor). Missing height defaults to 175/162/168 cm.
pub fn bmr_kcal_day(p: &UserProfile) -> f64 {
    let h = p.height_cm.unwrap_or(match p.sex {
        Sex::Male => 175.0,
        Sex::Female => 162.0,
        Sex::Other => 168.0,
    });
    let base = 10.0 * p.weight_kg + 6.25 * h - 5.0 * p.age;
    base + match p.sex {
        Sex::Male => 5.0,
        Sex::Female => -161.0,
        Sex::Other => -78.0,
    }
}

/// Keytel et al. (2005) HR-based energy expenditure, kcal/min (may be negative at low HR).
pub fn keytel_kcal_min(p: &UserProfile, bpm: f64) -> f64 {
    let male = (-55.0969 + 0.6309 * bpm + 0.1988 * p.weight_kg + 0.2017 * p.age) / 4.184;
    let female = (-20.4022 + 0.4472 * bpm - 0.1263 * p.weight_kg + 0.074 * p.age) / 4.184;
    match p.sex {
        Sex::Male => male,
        Sex::Female => female,
        Sex::Other => (male + female) / 2.0,
    }
}

/// kcal/min at a given HR: resting (BMR) below `RHR + 0.30 x HRR`, else Keytel (never below BMR).
pub fn kcal_per_min(p: &UserProfile, bpm: f64, zones: &HrZones) -> f64 {
    let bmr = bmr_kcal_day(p) / 1440.0;
    let thr = zones.resting_hr + 0.30 * (zones.max_hr - zones.resting_hr);
    if bpm < thr { bmr } else { keytel_kcal_min(p, bpm).max(bmr) }
}

/// Estimated kilocalories over `[start,end)`: only time with HR data is counted.
pub fn calories(hr: &[HrSample], p: &UserProfile, zones: &HrZones, start: i64, end: i64) -> f64 {
    let s = hr_in(hr, start, end);
    let d = sample_durations(s);
    s.iter().zip(d).map(|(x, dt)| kcal_per_min(p, x.bpm, zones) * dt / 60.0).sum()
}

/// Steps in `[start,end)` from strap step samples. `None` when the strap supplied no step data;
/// steps are not inferred from motion (a 1 Hz motion magnitude cannot resolve footfalls).
pub fn steps_total(steps: &[StepSample], start: i64, end: i64) -> Option<i64> {
    let a = lower_bound(steps, start, |s| s.ts_ms);
    let b = lower_bound(steps, end, |s| s.ts_ms);
    let sl = &steps[a..b.max(a)];
    if sl.is_empty() { None } else { Some(sl.iter().map(|s| s.steps as i64).sum()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_hr_rules() {
        let mut p = UserProfile { age: 30.0, ..Default::default() };
        // 208 - 0.7*30 = 187
        assert!((max_hr(&p, &[]) - 187.0).abs() < 1e-9);
        // observed 195 sustained wins once >= 600 samples
        let obs: Vec<_> = (0..1000).map(|i| HrSample { ts_ms: i * 1000, bpm: 195.0 }).collect();
        assert!((max_hr(&p, &obs) - 195.0).abs() < 1e-9);
        // too few samples -> Tanaka
        assert!((max_hr(&p, &obs[..100]) - 187.0).abs() < 1e-9);
        p.max_hr_override = Some(180.0);
        assert_eq!(max_hr(&p, &obs), 180.0);
    }

    #[test]
    fn zones_hand_worked() {
        // max 190, rest 50 -> reserve 140: zone starts 50+70=120, 134, 148, 162, 176
        let z = hr_zones(190.0, 50.0);
        let expect = [120.0, 134.0, 148.0, 162.0, 176.0];
        for (a, b) in z.lower_bpm.iter().zip(expect) {
            assert!((a - b).abs() < 1e-9);
        }
        assert_eq!(z.zone(100.0), 0);
        assert_eq!(z.zone(120.0), 1);
        assert_eq!(z.zone(150.0), 3);
        assert_eq!(z.zone(185.0), 5);
        assert!((z.pct_hrr(120.0) - 50.0).abs() < 1e-9);
    }

    #[test]
    fn time_in_zones_counts_minutes() {
        let z = hr_zones(190.0, 50.0);
        // 10 minutes at 1 Hz at 150 bpm (zone 3) then 5 min at 100 (zone 0)
        let mut hr = Vec::new();
        for i in 0..600 {
            hr.push(HrSample { ts_ms: i * 1000, bpm: 150.0 });
        }
        for i in 600..900 {
            hr.push(HrSample { ts_ms: i * 1000, bpm: 100.0 });
        }
        let m = time_in_zones(&hr, &z, 0, 900_000);
        assert!((m[2] - 10.0).abs() < 0.05, "{m:?}");
        assert!(m[0] + m[1] + m[3] + m[4] < 1e-9);
    }

    #[test]
    fn gaps_are_not_filled() {
        let z = hr_zones(190.0, 50.0);
        // two samples 10 minutes apart: each credited at most 15 s
        let hr = vec![HrSample { ts_ms: 0, bpm: 150.0 }, HrSample { ts_ms: 600_000, bpm: 150.0 }];
        let m = time_in_zones(&hr, &z, 0, 700_000);
        assert!(m[2] <= 0.51, "{m:?}"); // 15 s + median(15 s) = 0.5 min
    }

    #[test]
    fn keytel_hand_worked() {
        // male 30 y, 80 kg, 150 bpm: (-55.0969 + 94.635 + 15.904 + 6.051)/4.184 = 61.4931/4.184 = 14.697 kcal/min
        let p = UserProfile { age: 30.0, sex: Sex::Male, weight_kg: 80.0, ..Default::default() };
        assert!((keytel_kcal_min(&p, 150.0) - 14.697).abs() < 0.01);
        // female 30 y, 60 kg, 150 bpm: (-20.4022 + 67.08 - 7.578 + 2.22)/4.184 = 41.3198/4.184 = 9.876
        let f = UserProfile { age: 30.0, sex: Sex::Female, weight_kg: 60.0, ..Default::default() };
        assert!((keytel_kcal_min(&f, 150.0) - 9.876).abs() < 0.01);
    }

    #[test]
    fn bmr_and_resting_calories() {
        // Mifflin male 30y 80kg 175cm: 800 + 1093.75 - 150 + 5 = 1748.75 kcal/day
        let p = UserProfile { age: 30.0, sex: Sex::Male, weight_kg: 80.0, height_cm: Some(175.0), max_hr_override: None };
        assert!((bmr_kcal_day(&p) - 1748.75).abs() < 1e-9);
        let z = hr_zones(190.0, 50.0);
        // one hour at 60 bpm (below threshold 50+0.3*140 = 92): resting rate 1748.75/24 = 72.86 kcal
        let hr: Vec<_> = (0..3600).map(|i| HrSample { ts_ms: i * 1000, bpm: 60.0 }).collect();
        let k = calories(&hr, &p, &z, 0, 3_600_000);
        assert!((k - 72.86).abs() < 0.2, "{k}");
    }

    #[test]
    fn steps_only_from_strap() {
        assert_eq!(steps_total(&[], 0, 100), None);
        let s = [StepSample { ts_ms: 10, steps: 5 }, StepSample { ts_ms: 20, steps: 7 }, StepSample { ts_ms: 200, steps: 9 }];
        assert_eq!(steps_total(&s, 0, 100), Some(12));
    }
}
