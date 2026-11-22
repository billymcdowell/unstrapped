//! Raw measurements: R-R cleaning, RMSSD, resting HR, respiratory rate, skin temp, SpO2.

use super::types::*;
use super::util::*;

pub const RR_MIN_MS: f64 = 300.0;
pub const RR_MAX_MS: f64 = 2000.0;
/// A beat deviating more than this fraction from the local median is treated as ectopic/artifact.
pub const ECTOPIC_FRAC: f64 = 0.20;
/// Consecutive R-R values further apart than this (ms of timestamp) are not "successive".
pub const RR_GAP_MS: i64 = 5_000;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CleanedRr {
    pub kept: Vec<RrInterval>,
    pub n_input: usize,
    pub removed_range: usize,
    pub removed_ectopic: usize,
}

/// Artifact removal: (1) drop intervals outside 300-2000 ms; (2) drop beats that differ by more than
/// 20% from the median of the surrounding 5 beats (Malik-style rule). Input must be time-ordered.
pub fn clean_rr(rr: &[RrInterval]) -> CleanedRr {
    let in_range: Vec<RrInterval> = rr
        .iter()
        .copied()
        .filter(|r| r.rr_ms.is_finite() && (RR_MIN_MS..=RR_MAX_MS).contains(&r.rr_ms))
        .collect();
    let removed_range = rr.len() - in_range.len();
    let mut kept = Vec::with_capacity(in_range.len());
    for i in 0..in_range.len() {
        let lo = i.saturating_sub(2);
        let hi = (i + 3).min(in_range.len());
        if hi - lo < 3 {
            kept.push(in_range[i]);
            continue;
        }
        let w: Vec<f64> = in_range[lo..hi].iter().map(|r| r.rr_ms).collect();
        let med = median(&w).unwrap_or(in_range[i].rr_ms);
        if (in_range[i].rr_ms - med).abs() / med <= ECTOPIC_FRAC {
            kept.push(in_range[i]);
        }
    }
    CleanedRr {
        n_input: rr.len(),
        removed_range,
        removed_ectopic: in_range.len() - kept.len(),
        kept,
    }
}

/// RMSSD (ms) of successive differences: sqrt(mean((rr[i+1]-rr[i])^2)). Pairs separated by a
/// timestamp gap > 5 s are skipped. `None` with fewer than 2 usable pairs.
pub fn rmssd_rr(rr: &[RrInterval]) -> Option<f64> {
    let mut sum = 0.0;
    let mut n = 0usize;
    for w in rr.windows(2) {
        if w[1].ts_ms - w[0].ts_ms <= RR_GAP_MS {
            sum += (w[1].rr_ms - w[0].rr_ms).powi(2);
            n += 1;
        }
    }
    if n < 2 { None } else { Some((sum / n as f64).sqrt()) }
}

/// RMSSD on a plain value series (no timestamps).
pub fn rmssd_values(v: &[f64]) -> Option<f64> {
    if v.len() < 3 {
        return None;
    }
    let s: f64 = v.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum();
    Some((s / (v.len() - 1) as f64).sqrt())
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct HrvResult {
    pub rmssd_ms: f64,
    pub n_input: usize,
    pub n_clean: usize,
}

/// HRV (RMSSD) of an R-R slice after cleaning; requires at least `min_beats` clean beats.
pub fn hrv_rmssd(rr: &[RrInterval], min_beats: usize) -> Option<HrvResult> {
    let c = clean_rr(rr);
    if c.kept.len() < min_beats {
        return None;
    }
    rmssd_rr(&c.kept).map(|r| HrvResult { rmssd_ms: r, n_input: c.n_input, n_clean: c.kept.len() })
}

/// R-R slice with `start <= ts < end` (input sorted by time).
pub fn rr_in(rr: &[RrInterval], start: i64, end: i64) -> &[RrInterval] {
    let a = lower_bound(rr, start, |r| r.ts_ms);
    let b = lower_bound(rr, end, |r| r.ts_ms);
    &rr[a..b.max(a)]
}

pub fn hr_in(hr: &[HrSample], start: i64, end: i64) -> &[HrSample] {
    let a = lower_bound(hr, start, |r| r.ts_ms);
    let b = lower_bound(hr, end, |r| r.ts_ms);
    &hr[a..b.max(a)]
}

pub fn motion_in(m: &[MotionSample], start: i64, end: i64) -> &[MotionSample] {
    let a = lower_bound(m, start, |r| r.ts_ms);
    let b = lower_bound(m, end, |r| r.ts_ms);
    &m[a..b.max(a)]
}

/// Tumbling-window RMSSD: `(window_mid_ts, rmssd)` for each `window_ms` window in `[start,end)`
/// holding at least `min_beats` clean beats. Each window is cleaned independently.
pub fn windowed_rmssd(rr: &[RrInterval], start: i64, end: i64, window_ms: i64, min_beats: usize) -> Vec<(i64, f64)> {
    let mut out = Vec::new();
    let mut t = start;
    while t < end {
        let e = (t + window_ms).min(end);
        if let Some(h) = hrv_rmssd(rr_in(rr, t, e), min_beats) {
            out.push((t + (e - t) / 2, h.rmssd_ms));
        }
        t += window_ms;
    }
    out
}

/// Resting HR: the lowest *steady* stretch. HR in `[start,end)` is binned into 5-minute bins; a bin
/// qualifies if it holds >= 5 samples, mean >= 25 bpm and standard deviation <= 8 bpm. Returns the
/// minimum qualifying bin mean (rejects single-sample dips).
pub fn resting_hr(hr: &[HrSample], start: i64, end: i64) -> Option<f64> {
    let mut best: Option<f64> = None;
    let mut t = start;
    while t < end {
        let e = (t + 5 * MS_PER_MIN).min(end);
        let v: Vec<f64> = hr_in(hr, t, e).iter().map(|s| s.bpm).filter(|b| *b >= 25.0).collect();
        if v.len() >= 5 {
            let m = v.iter().sum::<f64>() / v.len() as f64;
            let sd = std_dev(&v).unwrap_or(0.0);
            if sd <= 8.0 && best.is_none_or(|b| m < b) {
                best = Some(m);
            }
        }
        t += 5 * MS_PER_MIN;
    }
    best
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct RespEstimate {
    pub rate_bpm: f64,
    /// Fraction of 0.1-0.5 Hz spectral power at the dominant peak (0..1): high = regular breathing.
    pub regularity: f64,
}

/// Respiratory rate from respiratory sinus arrhythmia in a window of R-R intervals (~1-3 minutes).
///
/// The beat series is split at timestamp gaps > 5 s and the longest chunk used; its tachogram is
/// resampled at 2 Hz (beat times rebuilt from cumulative R-R), detrended, Hann-windowed, and scanned
/// with a Goertzel filter bank over 0.10-0.50 Hz (6-30 breaths/min).
pub fn respiratory_rate_window(rr: &[RrInterval]) -> Option<RespEstimate> {
    let c = clean_rr(rr);
    let beats = &c.kept;
    if beats.len() < 30 {
        return None;
    }
    // longest chunk without a gap
    let (mut best_a, mut best_len, mut a) = (0usize, 0usize, 0usize);
    for i in 1..=beats.len() {
        if i == beats.len() || beats[i].ts_ms - beats[i - 1].ts_ms > RR_GAP_MS {
            if i - a > best_len {
                best_len = i - a;
                best_a = a;
            }
            a = i;
        }
    }
    let chunk = &beats[best_a..best_a + best_len];
    if chunk.len() < 30 {
        return None;
    }
    // beat times from cumulative R-R
    let mut t = Vec::with_capacity(chunk.len());
    let mut acc = 0.0;
    for r in chunk {
        acc += r.rr_ms / 1000.0;
        t.push(acc);
    }
    let span = t[t.len() - 1] - t[0];
    if span < 60.0 {
        return None;
    }
    const FS: f64 = 2.0;
    let n = (span * FS) as usize;
    if n < 64 {
        return None;
    }
    let mut y = Vec::with_capacity(n);
    let mut j = 0usize;
    for k in 0..n {
        let tk = t[0] + k as f64 / FS;
        while j + 2 < t.len() && t[j + 1] < tk {
            j += 1;
        }
        let (t0, t1) = (t[j], t[j + 1]);
        let f = if t1 > t0 { ((tk - t0) / (t1 - t0)).clamp(0.0, 1.0) } else { 0.0 };
        y.push(chunk[j].rr_ms * (1.0 - f) + chunk[j + 1].rr_ms * f);
    }
    // remove linear trend
    let nf = n as f64;
    let (mut sx, mut sy, mut sxx, mut sxy) = (0.0, 0.0, 0.0, 0.0);
    for (i, v) in y.iter().enumerate() {
        let x = i as f64;
        sx += x;
        sy += v;
        sxx += x * x;
        sxy += x * v;
    }
    let slope = (nf * sxy - sx * sy) / (nf * sxx - sx * sx);
    let icpt = (sy - slope * sx) / nf;
    for (i, v) in y.iter_mut().enumerate() {
        let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / (nf - 1.0)).cos();
        *v = (*v - (icpt + slope * i as f64)) * w;
    }
    let mut powers = Vec::new();
    let mut f = 0.10;
    while f <= 0.50 + 1e-9 {
        let w = std::f64::consts::TAU * f / FS;
        let coeff = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0, 0.0);
        for v in &y {
            let s0 = v + coeff * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        powers.push((f, s1 * s1 + s2 * s2 - coeff * s1 * s2));
        f += 0.005;
    }
    let total: f64 = powers.iter().map(|p| p.1).sum();
    if total <= 0.0 {
        return None;
    }
    let peak = powers.iter().copied().fold((0.0, f64::MIN), |a, p| if p.1 > a.1 { p } else { a });
    // sum power in +-0.02 Hz around the peak
    let near: f64 = powers.iter().filter(|p| (p.0 - peak.0).abs() <= 0.02).map(|p| p.1).sum();
    Some(RespEstimate { rate_bpm: peak.0 * 60.0, regularity: (near / total).clamp(0.0, 1.0) })
}

/// Overnight respiratory rate: median of 2-minute-window estimates over `[start,end)` (windows with
/// regularity below 0.15 are ignored as noise).
pub fn overnight_resp_rate(rr: &[RrInterval], start: i64, end: i64) -> Option<f64> {
    let mut v = Vec::new();
    let mut t = start;
    while t + 2 * MS_PER_MIN <= end {
        if let Some(e) = respiratory_rate_window(rr_in(rr, t, t + 2 * MS_PER_MIN)).filter(|e| e.regularity >= 0.15) {
            v.push(e.rate_bpm);
        }
        t += 2 * MS_PER_MIN;
    }
    if v.len() < 5 { None } else { median(&v) }
}

/// Median skin temperature (deg C) in `[start,end)`; ignores implausible values outside 20-42.
pub fn median_skin_temp(temp: &[TempSample], start: i64, end: i64) -> Option<f64> {
    let a = lower_bound(temp, start, |s| s.ts_ms);
    let b = lower_bound(temp, end, |s| s.ts_ms);
    let v: Vec<f64> = temp[a..b.max(a)].iter().map(|s| s.celsius).filter(|c| (20.0..=42.0).contains(c)).collect();
    if v.len() < 5 { None } else { median(&v) }
}

/// Skin temperature deviation from the personal baseline mean, in deg C (positive = warmer).
pub fn skin_temp_deviation(value_c: f64, baseline_mean_c: f64) -> f64 {
    value_c - baseline_mean_c
}

/// SpO2 passthrough: median percent in `[start,end)` of plausible readings (70-100).
pub fn spo2_median(spo2: &[Spo2Sample], start: i64, end: i64) -> Option<f64> {
    let a = lower_bound(spo2, start, |s| s.ts_ms);
    let b = lower_bound(spo2, end, |s| s.ts_ms);
    let v: Vec<f64> = spo2[a..b.max(a)].iter().map(|s| s.percent).filter(|p| (70.0..=100.0).contains(p)).collect();
    if v.len() < 3 { None } else { median(&v) }
}

#[cfg(test)]
pub(crate) mod testgen {
    use super::*;
    /// Deterministic xorshift for synthetic data.
    pub struct Rng(pub u64);
    impl Rng {
        pub fn next(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
        /// roughly N(0,1) (sum of 12 uniforms)
        pub fn gauss(&mut self) -> f64 {
            (0..12).map(|_| self.next()).sum::<f64>() - 6.0
        }
    }
    /// R-R series with a respiratory oscillation: rr = 60000/hr + amp*sin(2 pi f t) + noise.
    pub fn rsa_series(start: i64, secs: i64, hr: f64, breaths_per_min: f64, amp: f64, noise: f64, rng: &mut Rng) -> Vec<RrInterval> {
        let mut out = Vec::new();
        let mut t = 0.0f64; // seconds
        let f = breaths_per_min / 60.0;
        while t < secs as f64 {
            let rr = 60000.0 / hr + amp * (std::f64::consts::TAU * f * t).sin() + noise * rng.gauss();
            t += rr / 1000.0;
            out.push(RrInterval { ts_ms: start + (t * 1000.0) as i64, rr_ms: rr });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::testgen::*;
    use super::*;

    fn rr(ts: i64, v: f64) -> RrInterval {
        RrInterval { ts_ms: ts, rr_ms: v }
    }

    #[test]
    fn rmssd_hand_worked() {
        // rr = 800, 810, 790, 820 -> diffs 10, -20, 30 -> squares 100+400+900 = 1400 -> /3 = 466.67 -> sqrt = 21.60
        let v = [800.0, 810.0, 790.0, 820.0];
        assert!((rmssd_values(&v).unwrap() - 21.6025).abs() < 1e-3);
        let s: Vec<_> = v.iter().enumerate().map(|(i, x)| rr(i as i64 * 800, *x)).collect();
        assert!((rmssd_rr(&s).unwrap() - 21.6025).abs() < 1e-3);
    }

    #[test]
    fn cleaning_removes_range_and_ectopic() {
        let mut s: Vec<_> = (0..40).map(|i| rr(i * 1000, 1000.0 + (i % 2) as f64 * 10.0)).collect();
        s[10].rr_ms = 250.0; // out of range
        s[20].rr_ms = 1500.0; // 50% above neighbours: ectopic
        s[30].rr_ms = 2400.0; // out of range
        let c = clean_rr(&s);
        assert_eq!(c.removed_range, 2);
        assert_eq!(c.removed_ectopic, 1);
        assert_eq!(c.kept.len(), 37);
        // after cleaning RMSSD is ~10 ms (alternating +-10), not hundreds
        assert!(rmssd_rr(&c.kept).unwrap() < 25.0);
    }

    #[test]
    fn hrv_needs_enough_beats() {
        let s: Vec<_> = (0..10).map(|i| rr(i * 1000, 1000.0)).collect();
        assert!(hrv_rmssd(&s, 20).is_none());
    }

    #[test]
    fn resting_hr_is_lowest_steady_bin() {
        // 10 minutes at 60 bpm then 10 min at 50 bpm, one-sample dip to 30 in the first stretch.
        let mut hr = Vec::new();
        for i in 0..600 {
            hr.push(HrSample { ts_ms: i * 1000, bpm: 60.0 });
        }
        for i in 600..1200 {
            hr.push(HrSample { ts_ms: i * 1000, bpm: 50.0 });
        }
        hr[100].bpm = 30.0;
        assert!((resting_hr(&hr, 0, 1_200_000).unwrap() - 50.0).abs() < 1e-9);
    }

    #[test]
    fn resp_rate_recovers_breathing_frequency() {
        let mut rng = Rng(42);
        let s = rsa_series(0, 150, 60.0, 15.0, 40.0, 8.0, &mut rng);
        let e = respiratory_rate_window(&s).unwrap();
        assert!((e.rate_bpm - 15.0).abs() < 1.0, "got {}", e.rate_bpm);
        assert!(e.regularity > 0.3);
        let s2 = rsa_series(0, 150, 60.0, 12.0, 40.0, 8.0, &mut rng);
        assert!((respiratory_rate_window(&s2).unwrap().rate_bpm - 12.0).abs() < 1.0);
    }

    #[test]
    fn overnight_resp_is_median() {
        let mut rng = Rng(7);
        let s = rsa_series(0, 1200, 55.0, 14.0, 35.0, 8.0, &mut rng);
        let r = overnight_resp_rate(&s, 0, 1_200_000).unwrap();
        assert!((r - 14.0).abs() < 1.0, "got {r}");
    }

    #[test]
    fn skin_temp_and_spo2() {
        let t: Vec<_> = (0..10).map(|i| TempSample { ts_ms: i * 60_000, celsius: 33.0 + (i % 3) as f64 * 0.1 }).collect();
        // values 33.0,33.1,33.2 repeating: median of 10 = 33.1
        let m = median_skin_temp(&t, 0, 600_000).unwrap();
        assert!((m - 33.1).abs() < 1e-9);
        assert!((skin_temp_deviation(m, 32.8) - 0.3).abs() < 1e-9);
        let s: Vec<_> = [96.0, 97.0, 98.0, 40.0].iter().enumerate().map(|(i, p)| Spo2Sample { ts_ms: i as i64, percent: *p }).collect();
        assert_eq!(spo2_median(&s, 0, 10), Some(97.0)); // 40 rejected
    }
}
