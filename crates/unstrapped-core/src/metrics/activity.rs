//! Activity auto-detection: sustained raised heart rate outside sleep.

use super::hr::*;
use super::records::Activity;
use super::signals::{hr_in, motion_in};
use super::strain::*;
use super::types::*;
use super::util::*;

/// A minute is "active" when its mean HR is at least `RHR + 0.35 x HRR`.
pub const ACTIVITY_HRR_FRAC: f64 = 0.35;
/// Shortest activity reported.
pub const MIN_ACTIVITY_MIN: usize = 10;
/// Inactive/no-data minutes tolerated inside one activity.
pub const MAX_INTERNAL_GAP_MIN: usize = 3;
/// With motion data present, a minute needs mean motion >= this (g) to count (excludes stress/caffeine HR).
pub const ACTIVITY_MIN_MOTION_G: f64 = 0.02;

fn in_any(ranges: &[(i64, i64)], t0: i64, t1: i64) -> bool {
    ranges.iter().any(|(a, b)| t0 < *b && t1 > *a)
}

/// Detect activities in `[start,end)`, ignoring minutes overlapping `exclude` (sleep windows).
pub fn detect_activities(
    hr: &[HrSample],
    motion: &[MotionSample],
    exclude: &[(i64, i64)],
    zones: &HrZones,
    profile: &UserProfile,
    start: i64,
    end: i64,
) -> Vec<Activity> {
    let thr = zones.resting_hr + ACTIVITY_HRR_FRAC * (zones.max_hr - zones.resting_hr);
    let n = ((end - start) / MS_PER_MIN).max(0) as usize;
    // 0 = no data, 1 = inactive, 2 = active
    let mut label = vec![0u8; n];
    for (i, l) in label.iter_mut().enumerate() {
        let t0 = start + i as i64 * MS_PER_MIN;
        let t1 = t0 + MS_PER_MIN;
        if in_any(exclude, t0, t1) {
            continue;
        }
        let h = hr_in(hr, t0, t1);
        if h.len() < 3 {
            continue;
        }
        let mean_hr = h.iter().map(|s| s.bpm).sum::<f64>() / h.len() as f64;
        let m = motion_in(motion, t0, t1);
        let moving = m.is_empty() || m.iter().map(|s| s.magnitude_g).sum::<f64>() / m.len() as f64 >= ACTIVITY_MIN_MOTION_G;
        *l = if mean_hr >= thr && moving { 2 } else { 1 };
    }
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut cur: Option<(usize, usize)> = None; // (first_active, last_active)
    let mut gap = 0usize;
    for (i, l) in label.iter().enumerate() {
        if *l == 2 {
            cur = Some(match cur {
                Some((a, _)) => (a, i),
                None => (i, i),
            });
            gap = 0;
        } else if let Some(c) = cur {
            gap += 1;
            if gap > MAX_INTERNAL_GAP_MIN {
                runs.push(c);
                cur = None;
            }
        }
    }
    if let Some(c) = cur {
        runs.push(c);
    }
    runs.into_iter()
        .filter(|(a, b)| b - a + 1 >= MIN_ACTIVITY_MIN)
        .map(|(a, b)| {
            let s = start + a as i64 * MS_PER_MIN;
            let e = start + (b as i64 + 1) * MS_PER_MIN;
            let h = hr_in(hr, s, e);
            let avg = h.iter().map(|x| x.bpm).sum::<f64>() / h.len().max(1) as f64;
            let mx = h.iter().map(|x| x.bpm).fold(0.0, f64::max);
            let zm = time_in_zones(hr, zones, s, e);
            Activity {
                id: format!("act-{s}"),
                start_ms: s,
                end_ms: e,
                kind: "Activity".to_string(),
                auto_detected: true,
                strain: strain_for_window(hr, zones, s, e),
                avg_hr: avg,
                max_hr: mx,
                calories: calories(hr, profile, zones, s, e),
                zone1_min: zm[0].round() as i64,
                zone2_min: zm[1].round() as i64,
                zone3_min: zm[2].round() as i64,
                zone4_min: zm[3].round() as i64,
                zone5_min: zm[4].round() as i64,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series(segs: &[(i64, i64, f64)]) -> Vec<HrSample> {
        let mut v = Vec::new();
        for (a, b, bpm) in segs {
            for s in *a..*b {
                v.push(HrSample { ts_ms: s * 1000, bpm: *bpm });
            }
        }
        v
    }

    #[test]
    fn detects_a_run_and_ignores_a_blip() {
        let z = hr_zones(190.0, 50.0); // threshold 50 + 0.35*140 = 99
        let p = UserProfile { age: 30.0, sex: Sex::Male, weight_kg: 75.0, ..Default::default() };
        // 0-30 min at 70; 30-60 min at 150 (zone 3); 60-62 min blip at 130; 62-90 at 70
        let hr = series(&[(0, 1800, 70.0), (1800, 3600, 150.0), (3600, 3720, 130.0), (3720, 5400, 70.0)]);
        let acts = detect_activities(&hr, &[], &[], &z, &p, 0, 5_400_000);
        // run of 30 min + 2 min blip within 3-minute gap? blip starts right after run (no gap) -> merged: 32 min
        assert_eq!(acts.len(), 1);
        let a = &acts[0];
        assert_eq!(a.start_ms, 1_800_000);
        assert_eq!(a.end_ms, 3_720_000);
        assert!(a.max_hr >= 150.0 && a.avg_hr > 140.0);
        // 30 min zone 3 -> 30*3 = 90 + blip 2 min zone 2 (130>=134? no: zone1 120..134) -> 2 min zone 1 = 2 -> load 92
        // strain = 21 ln(93)/ln(7201) = 10.71
        assert!((a.strain - 10.71).abs() < 0.1, "{}", a.strain);
        assert_eq!(a.zone3_min, 30);
    }

    #[test]
    fn short_bouts_and_sleep_are_excluded() {
        let z = hr_zones(190.0, 50.0);
        let p = UserProfile::default();
        let hr = series(&[(0, 600, 70.0), (600, 1000, 150.0), (1000, 3000, 70.0)]); // 6-min bout
        assert!(detect_activities(&hr, &[], &[], &z, &p, 0, 3_000_000).is_empty());
        let hr2 = series(&[(0, 1800, 150.0)]);
        assert!(detect_activities(&hr2, &[], &[(0, 1_800_000)], &z, &p, 0, 1_800_000).is_empty());
    }

    #[test]
    fn still_elevated_hr_is_not_an_activity() {
        let z = hr_zones(190.0, 50.0);
        let p = UserProfile::default();
        let hr = series(&[(0, 1800, 120.0)]);
        let still: Vec<_> = (0..1800).map(|s| MotionSample { ts_ms: s * 1000, magnitude_g: 0.002 }).collect();
        assert!(detect_activities(&hr, &still, &[], &z, &p, 0, 1_800_000).is_empty());
        let moving: Vec<_> = (0..1800).map(|s| MotionSample { ts_ms: s * 1000, magnitude_g: 0.3 }).collect();
        assert_eq!(detect_activities(&hr, &moving, &[], &z, &p, 0, 1_800_000).len(), 1);
    }
}
