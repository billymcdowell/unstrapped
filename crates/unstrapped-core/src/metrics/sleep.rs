//! Sleep/wake detection, main sleep vs naps, 30 s epoch staging, disturbances, latency, efficiency.

use super::records::*;
use super::signals::*;
use super::types::*;
use super::util::*;

pub const EPOCH_MS: i64 = 30_000;
/// Epoch is a movement epoch when this fraction of its motion samples exceed [`MOVE_G`].
pub const MOVE_FRAC_WAKE: f64 = 0.15;
pub const MOVE_FRAC_STILL: f64 = 0.10;
pub const MOVE_G: f64 = 0.05;
/// Detection: an epoch is "sleepy" when HR <= `SLEEP_HR_MULT` x (10th percentile of HR in the search window)
/// and mean motion < `SLEEP_MOTION_G`.
pub const SLEEP_HR_MULT: f64 = 1.25;
pub const SLEEP_MOTION_G: f64 = 0.03;
/// Wake periods shorter than this between two sleep runs are merged into one session.
pub const MERGE_GAP_MIN: usize = 30;
/// Sessions shorter than this are dropped.
pub const MIN_SESSION_MIN: i64 = 20;
/// Sessions at least this long (and ending on the day) can be the main sleep.
pub const MIN_MAIN_SLEEP_MIN: i64 = 180;
/// A wake run of at least this many epochs after onset is a disturbance.
pub const DISTURBANCE_MIN_EPOCHS: usize = 2;
const IN_BED_MAX_BACK_MIN: usize = 45;
/// Lying-still motion ceiling (g) for the pre-sleep in-bed extension (needs motion data).
const IN_BED_STILL_G: f64 = 0.015;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SleepWindow {
    /// First sleepy epoch (sleep onset).
    pub onset_ms: i64,
    pub end_ms: i64,
    /// Start of the lying-still period before onset (in-bed start). `<= onset_ms`.
    pub bed_start_ms: i64,
}

struct Grid {
    start: i64,
    n: usize,
    hr: Vec<Option<f64>>,
    motion: Vec<Option<f64>>,
    move_frac: Vec<Option<f64>>,
}

fn build_grid(hr: &[HrSample], motion: &[MotionSample], start: i64, end: i64) -> Grid {
    let n = ((end - start).max(0) / EPOCH_MS) as usize;
    let mut hs = vec![0.0; n];
    let mut hc = vec![0u32; n];
    let mut ms = vec![0.0; n];
    let mut mc = vec![0u32; n];
    let mut mh = vec![0u32; n];
    for s in hr_in(hr, start, start + n as i64 * EPOCH_MS) {
        if s.bpm.is_finite() && s.bpm > 20.0 {
            let i = ((s.ts_ms - start) / EPOCH_MS) as usize;
            hs[i] += s.bpm;
            hc[i] += 1;
        }
    }
    for s in motion_in(motion, start, start + n as i64 * EPOCH_MS) {
        let i = ((s.ts_ms - start) / EPOCH_MS) as usize;
        ms[i] += s.magnitude_g;
        mc[i] += 1;
        if s.magnitude_g > MOVE_G {
            mh[i] += 1;
        }
    }
    Grid {
        start,
        n,
        hr: (0..n).map(|i| (hc[i] > 0).then(|| hs[i] / hc[i] as f64)).collect(),
        motion: (0..n).map(|i| (mc[i] > 0).then(|| ms[i] / mc[i] as f64)).collect(),
        move_frac: (0..n).map(|i| (mc[i] > 0).then(|| mh[i] as f64 / mc[i] as f64)).collect(),
    }
}

/// Find sleep sessions in `[start,end)` from stillness + low HR.
///
/// 1. 30 s epochs are "sleepy" when mean HR <= 1.25 x the window's 10th-percentile HR and mean motion
///    < 0.03 g (HR alone is used where motion data is missing);
/// 2. an 11-epoch majority vote (>= 60%) removes blips;
/// 3. runs separated by < 30 min of wake (and < 10 min of missing data) are merged;
/// 4. sessions shorter than 20 min are dropped; in-bed start is found by walking back (max 45 min)
///    through very still (< 0.015 g), moderately low-HR epochs (needs motion data).
///
/// Needs >= 1 hour of HR data in the window, else returns nothing.
pub fn detect_sleep(hr: &[HrSample], motion: &[MotionSample], start: i64, end: i64) -> Vec<SleepWindow> {
    let g = build_grid(hr, motion, start, end);
    let vals: Vec<f64> = g.hr.iter().flatten().copied().collect();
    if vals.len() < 120 {
        return vec![];
    }
    let reference = percentile(&vals, 10.0).unwrap_or(60.0);
    let thr = reference * SLEEP_HR_MULT;
    // 0 nodata, 1 wake, 2 sleepy (raw)
    let raw: Vec<u8> = (0..g.n)
        .map(|i| match g.hr[i] {
            None => 0,
            Some(h) => {
                let still = g.motion[i].is_none_or(|m| m < SLEEP_MOTION_G);
                if h <= thr && still { 2 } else { 1 }
            }
        })
        .collect();
    let smooth: Vec<u8> = (0..g.n)
        .map(|i| {
            if raw[i] == 0 {
                return 0;
            }
            let lo = i.saturating_sub(5);
            let hi = (i + 6).min(g.n);
            let (mut s, mut t) = (0, 0);
            for r in &raw[lo..hi] {
                if *r != 0 {
                    t += 1;
                    if *r == 2 {
                        s += 1;
                    }
                }
            }
            if s as f64 / t as f64 >= 0.6 { 2 } else { 1 }
        })
        .collect();
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < g.n {
        if smooth[i] == 2 {
            let a = i;
            while i + 1 < g.n && smooth[i + 1] == 2 {
                i += 1;
            }
            runs.push((a, i));
        }
        i += 1;
    }
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for r in runs {
        if let Some(last) = merged.last_mut() {
            let gap = r.0 - last.1 - 1;
            let nodata = smooth[last.1 + 1..r.0].iter().filter(|x| **x == 0).count();
            // short wake blip, or a longer strap-off stretch (<= 3 h, mostly no data) bridged as one session;
            // the missing minutes stay unstaged gaps and are not counted as sleep.
            let blip = gap <= MERGE_GAP_MIN * 2 && nodata <= 20;
            let strap_off = gap <= 360 && nodata * 5 >= gap * 4;
            if blip || strap_off {
                last.1 = r.1;
                continue;
            }
        }
        merged.push(r);
    }
    let mut out: Vec<SleepWindow> = Vec::new();
    for (a, b) in merged {
        if ((b - a + 1) as i64) * EPOCH_MS < MIN_SESSION_MIN * MS_PER_MIN {
            continue;
        }
        let prev_end = out.last().map_or(0usize, |w| ((w.end_ms - start) / EPOCH_MS) as usize);
        let mut bed = a;
        let floor = a.saturating_sub(IN_BED_MAX_BACK_MIN * 2).max(prev_end);
        while bed > floor {
            let j = bed - 1;
            let ok = g.hr[j].is_some_and(|h| h <= reference * 1.4) && g.motion[j].is_some_and(|m| m < IN_BED_STILL_G);
            if !ok {
                break;
            }
            bed = j;
        }
        out.push(SleepWindow {
            onset_ms: start + a as i64 * EPOCH_MS,
            end_ms: start + (b as i64 + 1) * EPOCH_MS,
            bed_start_ms: start + bed as i64 * EPOCH_MS,
        });
    }
    out
}

/// Stage per 30 s epoch of `[bed_start, end)`; `None` = no HR data (gap).
pub fn stage_epochs(hr: &[HrSample], rr: &[RrInterval], motion: &[MotionSample], w: &SleepWindow) -> Vec<Option<Stage>> {
    let g = build_grid(hr, motion, w.bed_start_ms, w.end_ms);
    let n = g.n;
    let onset_idx = (((w.onset_ms - w.bed_start_ms) / EPOCH_MS) as usize).min(n);
    // features
    let hr_smooth: Vec<Option<f64>> = (0..n)
        .map(|i| {
            let v: Vec<f64> = g.hr[i.saturating_sub(5)..(i + 6).min(n)].iter().flatten().copied().collect();
            mean(&v)
        })
        .collect();
    let hr_var: Vec<Option<f64>> = (0..n)
        .map(|i| {
            let v: Vec<f64> = g.hr[i.saturating_sub(5)..(i + 6).min(n)].iter().flatten().copied().collect();
            if v.len() >= 4 { std_dev(&v) } else { None }
        })
        .collect();
    let rmssd: Vec<Option<f64>> = (0..n)
        .map(|i| {
            if i < onset_idx || g.hr[i].is_none() {
                return None;
            }
            let tc = g.start + i as i64 * EPOCH_MS + EPOCH_MS / 2;
            hrv_rmssd(rr_in(rr, tc - 150_000, tc + 150_000), 20).map(|h| h.rmssd_ms)
        })
        .collect();
    let resp: Vec<Option<RespEstimate>> = (0..n)
        .map(|i| {
            if i < onset_idx || g.hr[i].is_none() {
                return None;
            }
            let tc = g.start + i as i64 * EPOCH_MS + EPOCH_MS / 2;
            respiratory_rate_window(rr_in(rr, tc - 90_000, tc + 90_000))
        })
        .collect();
    let in_sleep = |v: &[Option<f64>]| -> Vec<f64> { v[onset_idx.min(n)..].iter().flatten().copied().collect() };
    let hr_e_sleep = in_sleep(&g.hr);
    let p25_hr = percentile(&in_sleep(&hr_smooth), 25.0);
    let p70_rmssd = percentile(&in_sleep(&rmssd), 70.0);
    let p65_var = percentile(&in_sleep(&hr_var), 65.0);
    let regs: Vec<f64> = resp[onset_idx.min(n)..].iter().flatten().map(|r| r.regularity).collect();
    let p50_reg = percentile(&regs, 50.0);
    let p40_reg = percentile(&regs, 40.0);
    let med_hr = percentile(&hr_e_sleep, 50.0);
    let p92_hr = percentile(&hr_e_sleep, 92.0);

    let mut st: Vec<Option<Stage>> = (0..n)
        .map(|i| {
            let h = g.hr[i]?;
            if i < onset_idx {
                return Some(Stage::Awake);
            }
            let moving = g.move_frac[i].is_some_and(|f| f >= MOVE_FRAC_WAKE);
            if moving && med_hr.is_none_or(|m| h >= m) {
                return Some(Stage::Awake);
            }
            if g.move_frac[i].is_none() && p92_hr.is_some_and(|p| h >= p) {
                return Some(Stage::Awake);
            }
            let still = g.move_frac[i].is_none_or(|f| f <= MOVE_FRAC_STILL);
            let hs = hr_smooth[i].unwrap_or(h);
            let low_hr = p25_hr.is_some_and(|p| hs <= p);
            let deep_resp = match (resp[i], p50_reg) {
                (Some(r), Some(p)) => r.regularity >= p,
                _ => true,
            };
            if still && low_hr && deep_resp && matches!((rmssd[i], p70_rmssd), (Some(r), Some(p)) if r >= p) {
                return Some(Stage::Deep);
            }
            let rem_resp = match (resp[i], p40_reg) {
                (Some(r), Some(p)) => r.regularity <= p,
                _ => med_hr.is_some_and(|m| hs >= m),
            };
            if still && !low_hr && rem_resp && matches!((hr_var[i], p65_var), (Some(v), Some(p)) if v >= p) {
                return Some(Stage::Rem);
            }
            Some(Stage::Light)
        })
        .collect();

    // 5-epoch mode smoothing (post-onset only; Awake from motion is kept as-is when it is a single epoch run >= 2)
    let orig = st.clone();
    for i in onset_idx..n {
        if orig[i].is_none() {
            continue;
        }
        let mut counts = [0usize; 4];
        for s in orig[i.saturating_sub(2)..(i + 3).min(n)].iter().flatten() {
            counts[*s as usize] += 1;
        }
        let cur = orig[i].unwrap();
        let (best, bc) = counts.iter().enumerate().fold((cur as usize, counts[cur as usize]), |a, (k, c)| if *c > a.1 { (k, *c) } else { a });
        let _ = bc;
        st[i] = Some(match best {
            0 => Stage::Awake,
            1 => Stage::Light,
            2 => Stage::Deep,
            _ => Stage::Rem,
        });
    }
    // physiology: no REM in the first 15 min; no deep in the last 40% of the night
    let span = n.saturating_sub(onset_idx);
    for (i, s) in st.iter_mut().enumerate().skip(onset_idx) {
        let k = i - onset_idx;
        if *s == Some(Stage::Rem) && k < 30 {
            *s = Some(Stage::Light);
        }
        if *s == Some(Stage::Deep) && k as f64 > span as f64 * 0.6 {
            *s = Some(Stage::Light);
        }
    }
    st
}

fn segments(stages: &[Option<Stage>], start: i64) -> Vec<StageSegment> {
    let mut out: Vec<StageSegment> = Vec::new();
    for (i, s) in stages.iter().enumerate() {
        let Some(s) = s else { continue };
        let t0 = start + i as i64 * EPOCH_MS;
        let t1 = t0 + EPOCH_MS;
        match out.last_mut() {
            Some(l) if l.stage == *s && l.end_ms == t0 => l.end_ms = t1,
            _ => out.push(StageSegment { start_ms: t0, end_ms: t1, stage: *s }),
        }
    }
    out
}

/// Full per-session analysis: stages, minutes per stage, disturbances, latency, efficiency, restorative %,
/// resting HR, sleeping HRV and respiratory rate. Need fields are left `None` (filled by the scorer).
pub fn analyze_sleep(w: &SleepWindow, hr: &[HrSample], rr: &[RrInterval], motion: &[MotionSample], is_nap: bool) -> Sleep {
    let stages = stage_epochs(hr, rr, motion, w);
    let onset_idx = (((w.onset_ms - w.bed_start_ms) / EPOCH_MS) as usize).min(stages.len());
    let count = |f: &dyn Fn(Stage) -> bool| stages.iter().flatten().filter(|s| f(**s)).count();
    let half = |c: usize| (c as f64 / 2.0).round() as i64;
    let awake = count(&|s| s == Stage::Awake);
    let light = count(&|s| s == Stage::Light);
    let deep = count(&|s| s == Stage::Deep);
    let rem = count(&|s| s == Stage::Rem);
    let gaps = stages.iter().filter(|s| s.is_none()).count();
    let mut disturbances = 0;
    let mut dist_ts = Vec::new();
    let mut run = 0usize;
    for i in onset_idx..=stages.len() {
        if i < stages.len() && stages[i] == Some(Stage::Awake) {
            run += 1;
        } else {
            if run >= DISTURBANCE_MIN_EPOCHS {
                disturbances += 1;
                dist_ts.push(w.bed_start_ms + (i - run) as i64 * EPOCH_MS);
            }
            run = 0;
        }
    }
    let asleep_e = light + deep + rem;
    let in_bed_min = ((w.end_ms - w.bed_start_ms) as f64 / MS_PER_MIN as f64).round() as i64;
    let asleep_min = half(asleep_e);
    // hrv: mean of 5-minute window RMSSDs, skipping windows that are mostly awake / gap
    let windows = windowed_rmssd(rr, w.onset_ms, w.end_ms, 5 * MS_PER_MIN, 30);
    let good: Vec<f64> = windows
        .iter()
        .filter(|(mid, _)| {
            let idx = ((mid - w.bed_start_ms) / EPOCH_MS) as usize;
            let lo = idx.saturating_sub(5);
            let hi = (idx + 5).min(stages.len());
            let bad = stages[lo..hi].iter().filter(|s| !matches!(s, Some(x) if *x != Stage::Awake)).count();
            bad * 2 <= hi - lo
        })
        .map(|(_, v)| *v)
        .collect();
    Sleep {
        id: format!("sleep-{}", w.bed_start_ms),
        start_ms: w.bed_start_ms,
        end_ms: w.end_ms,
        is_nap,
        in_bed_min,
        asleep_min,
        latency_min: ((w.onset_ms - w.bed_start_ms) / MS_PER_MIN).max(0),
        disturbances,
        awake_min: half(awake),
        light_min: half(light),
        deep_min: half(deep),
        rem_min: half(rem),
        need_baseline_min: None,
        need_strain_min: None,
        need_debt_min: None,
        need_naps_min: None,
        resp_rate: overnight_resp_rate(rr, w.onset_ms, w.end_ms),
        onset_ms: w.onset_ms,
        efficiency_pct: if in_bed_min > 0 { asleep_min as f64 / in_bed_min as f64 * 100.0 } else { 0.0 },
        restorative_pct: if asleep_e > 0 { (deep + rem) as f64 / asleep_e as f64 * 100.0 } else { 0.0 },
        resting_hr: resting_hr(hr, w.onset_ms, w.end_ms),
        hrv_ms: mean(&good),
        gap_min: half(gaps),
        stages: segments(&stages, w.bed_start_ms),
        disturbance_ts: dist_ts,
    }
}

#[cfg(test)]
mod tests {
    use super::testgen_pub::*;
    use super::*;

    const H: i64 = 3_600_000;

    #[test]
    fn detects_night_merges_blip_and_separates_nap() {
        // t in hours: 0-2 awake(70, moving), 2-2.5 lying (66, still), 2.5-9.5 sleep(50) with 5-min blip at 5h,
        // 9.5-12 awake, 12-12.5 nap (50, still), 12.5-14 awake.
        let mut hr = Vec::new();
        let mut mo = Vec::new();
        let mut rng = super::super::signals::testgen::Rng(5);
        for s in 0..(14 * 3600) {
            let h = s as f64 / 3600.0;
            let (bpm, m) = if h < 2.0 {
                (70.0, 0.12)
            } else if h < 2.5 {
                (66.0, 0.004)
            } else if h < 9.5 {
                if (5.0..5.0 + 5.0 / 60.0).contains(&h) { (78.0, 0.2) } else { (50.0, 0.004) }
            } else if h < 12.0 {
                (70.0, 0.12)
            } else if h < 12.5 {
                (50.0, 0.004)
            } else {
                (70.0, 0.12)
            };
            hr.push(HrSample { ts_ms: s as i64 * 1000, bpm: bpm + rng.gauss() * 0.8 });
            mo.push(MotionSample { ts_ms: s as i64 * 1000, magnitude_g: m });
        }
        let w = detect_sleep(&hr, &mo, 0, 14 * H);
        assert_eq!(w.len(), 2, "{w:?}");
        let night = w[0];
        assert!((night.onset_ms - (2.5 * H as f64) as i64).abs() < 12 * MS_PER_MIN, "onset {}", night.onset_ms / MS_PER_MIN);
        assert!((night.end_ms - (9.5 * H as f64) as i64).abs() < 12 * MS_PER_MIN);
        // in-bed starts in the lying-still period (2.0-2.5 h)
        assert!(night.bed_start_ms < night.onset_ms - 15 * MS_PER_MIN && night.bed_start_ms >= 2 * H - 6 * MS_PER_MIN);
        let nap = w[1];
        assert!((nap.end_ms - nap.onset_ms - 30 * MS_PER_MIN).abs() < 10 * MS_PER_MIN);
    }

    #[test]
    fn too_little_data_returns_nothing() {
        let hr: Vec<_> = (0..600).map(|s| HrSample { ts_ms: s * 1000, bpm: 50.0 }).collect();
        assert!(detect_sleep(&hr, &[], 0, 600_000).is_empty());
    }

    #[test]
    fn staging_finds_deep_and_rem_in_the_right_places() {
        let (hr, rr, mo, w, truth) = synthetic_night();
        let st = stage_epochs(&hr, &rr, &mo, &w);
        let mut hit = 0;
        let mut tot = 0;
        let mut rem_hit = 0;
        let mut rem_tot = 0;
        for (i, s) in st.iter().enumerate() {
            let t = w.bed_start_ms + i as i64 * EPOCH_MS + EPOCH_MS / 2;
            let tr = truth(t);
            if *s == Some(Stage::Deep) {
                tot += 1;
                if tr == Stage::Deep {
                    hit += 1;
                }
            }
            if *s == Some(Stage::Rem) {
                rem_tot += 1;
                if tr == Stage::Rem {
                    rem_hit += 1;
                }
            }
        }
        assert!(tot > 20, "deep epochs {tot}");
        assert!(hit as f64 / tot as f64 > 0.6, "deep precision {hit}/{tot}");
        assert!(rem_tot > 20, "rem epochs {rem_tot}");
        assert!(rem_hit as f64 / rem_tot as f64 > 0.5, "rem precision {rem_hit}/{rem_tot}");
    }

    #[test]
    fn analyze_sleep_accounts_for_every_minute() {
        let (hr, rr, mo, w, _) = synthetic_night();
        let s = analyze_sleep(&w, &hr, &rr, &mo, false);
        assert_eq!(s.in_bed_min, (w.end_ms - w.bed_start_ms) / MS_PER_MIN);
        assert!((s.awake_min + s.light_min + s.deep_min + s.rem_min + s.gap_min - s.in_bed_min).abs() <= 2);
        assert!(s.asleep_min > 300 && s.efficiency_pct > 85.0 && s.efficiency_pct <= 100.0);
        assert!(s.latency_min >= 15 && s.latency_min <= 45, "latency {}", s.latency_min);
        // restorative % = (deep+rem)/asleep
        let r = (s.deep_min + s.rem_min) as f64 / s.asleep_min as f64 * 100.0;
        assert!((r - s.restorative_pct).abs() < 2.0);
        assert!(s.resting_hr.unwrap() < 52.0);
        assert!(s.hrv_ms.unwrap() > 40.0);
        let rr_rate = s.resp_rate.unwrap();
        assert!((rr_rate - 14.5).abs() < 2.0, "resp {rr_rate}");
        // segments tile without overlap
        for p in s.stages.windows(2) {
            assert!(p[0].end_ms <= p[1].start_ms);
        }
    }

    #[test]
    fn disturbances_count_wake_runs_after_onset() {
        let (mut hr, rr, mut mo, w, _) = synthetic_night();
        // inject a 4-minute movement + HR spike at 3 h after onset
        let t0 = w.onset_ms + 3 * H;
        for s in hr.iter_mut().filter(|s| s.ts_ms >= t0 && s.ts_ms < t0 + 240_000) {
            s.bpm = 80.0;
        }
        for s in mo.iter_mut().filter(|s| s.ts_ms >= t0 && s.ts_ms < t0 + 240_000) {
            s.magnitude_g = 0.3;
        }
        let s = analyze_sleep(&w, &hr, &rr, &mo, false);
        assert!(s.disturbances >= 1, "disturbances {}", s.disturbances);
        assert!(s.disturbance_ts.iter().any(|t| (*t - t0).abs() < 3 * MS_PER_MIN));
    }
}

/// Synthetic-night generator shared by tests in this and other modules.
#[cfg(test)]
pub(crate) mod testgen_pub {
    use super::super::signals::testgen::Rng;
    use super::*;

    /// 8 h night: 30 min lying (HR 64), then 90-min cycles of [light 30, deep 25 (cycles 1-2) or light, rem 20, light]
    /// Returns (hr, rr, motion, window, truth(ts)->stage).
    #[allow(clippy::type_complexity)]
    pub fn synthetic_night() -> (Vec<HrSample>, Vec<RrInterval>, Vec<MotionSample>, SleepWindow, impl Fn(i64) -> Stage) {
        let mut rng = Rng(11);
        let bed = 0i64;
        let onset = 30 * MS_PER_MIN;
        let end = onset + 7 * 3_600_000;
        let truth = move |t: i64| -> Stage {
            if t < onset {
                return Stage::Awake;
            }
            let m = ((t - onset) / MS_PER_MIN) % 90;
            let cycle = (t - onset) / MS_PER_MIN / 90;
            match m {
                0..=29 => Stage::Light,
                30..=54 if cycle < 3 => Stage::Deep,
                30..=54 => Stage::Light,
                55..=74 => Stage::Rem,
                _ => Stage::Light,
            }
        };
        let mut hr = Vec::new();
        let mut mo = Vec::new();
        let mut rr = Vec::new();
        let mut hr_walk = 0.0f64;
        for s in 0..((end - bed) / 1000) {
            let t = bed + s * 1000;
            let st = truth(t);
            let lying = t < onset;
            hr_walk = 0.97 * hr_walk + rng.gauss() * 0.9; // slow random walk used for REM variability
            let base = match st {
                Stage::Awake => 64.0,
                Stage::Light => 52.0,
                Stage::Deep => 47.0,
                Stage::Rem => 55.0 + hr_walk * 1.6,
            };
            hr.push(HrSample { ts_ms: t, bpm: base + 0.6 * rng.gauss() });
            mo.push(MotionSample { ts_ms: t, magnitude_g: if lying { 0.004 } else { 0.003 } * (0.5 + rng.next()) });
        }
        // R-R beat series: amplitude/regularity per stage
        let mut t = 0.0f64;
        while (t as i64) < (end - bed) / 1000 {
            let tm = bed + (t * 1000.0) as i64;
            let st = truth(tm);
            let hrv = hr.get(((tm - bed) / 1000) as usize).map_or(55.0, |h| h.bpm);
            let (amp, bpm, noise) = match st {
                Stage::Awake => (22.0, 14.5, 15.0),
                Stage::Light => (35.0, 14.5, 10.0),
                Stage::Deep => (60.0, 13.0, 4.0),
                Stage::Rem => (22.0, 15.0, 30.0),
            };
            // REM: irregular breathing = frequency wobble
            let f = if st == Stage::Rem { (bpm + 3.0 * (t / 37.0).sin() + 2.0 * (t / 11.0).cos()) / 60.0 } else { bpm / 60.0 };
            let rrv = 60000.0 / hrv + amp * (std::f64::consts::TAU * f * t).sin() + noise * rng.gauss();
            t += rrv / 1000.0;
            rr.push(RrInterval { ts_ms: bed + (t * 1000.0) as i64, rr_ms: rrv });
        }
        let w = SleepWindow { onset_ms: onset, end_ms: end, bed_start_ms: bed };
        (hr, rr, mo, w, truth)
    }
}
