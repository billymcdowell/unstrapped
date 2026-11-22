//! Per-day orchestrator: raw samples + profile + history -> everything for the 9.4 tables.

use super::activity::*;
use super::baselines::*;
use super::hr::*;
use super::monitor::*;
use super::records::*;
use super::scores::*;
use super::signals::*;
use super::sleep::*;
use super::strain::*;
use super::stress::*;
use super::types::*;
use super::util::*;
use super::ALGO_VERSION;
use serde::{Deserialize, Serialize};

/// Raw sample streams. Each slice should be time-ordered (they are re-sorted defensively) and should span
/// at least `[local midnight of the day - 6 h, local midnight of the next day)`; extra data is ignored.
#[derive(Debug, Clone, Copy, Default)]
pub struct RawStreams<'a> {
    pub hr: &'a [HrSample],
    pub rr: &'a [RrInterval],
    pub motion: &'a [MotionSample],
    pub temp: &'a [TempSample],
    pub spo2: &'a [Spo2Sample],
    pub steps: &'a [StepSample],
}

#[derive(Debug, Clone, Copy)]
pub struct DayInput<'a> {
    /// Local day `YYYY-MM-DD`: the day the main sleep ends on, and the 24 h strain/activity window.
    pub day: &'a str,
    /// Minutes east of UTC for local time.
    pub tz_offset_min: i32,
    pub profile: &'a UserProfile,
    pub streams: RawStreams<'a>,
    /// Previously computed summaries (any order; only days before `day` are used; ~30-90 days is plenty).
    pub history: &'a [DailySummary],
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DayResult {
    pub day: String,
    /// `daily_summary` row.
    pub summary: DailySummary,
    /// `sleeps` rows (main sleep first, then naps), each with its `stages` (`sleep_stages` rows).
    pub sleeps: Vec<Sleep>,
    /// `activities` rows.
    pub activities: Vec<Activity>,
    /// `stress` rows for the local day (every 5 minutes, movement windows skipped).
    pub stress: Vec<StressPoint>,
    /// `baselines` rows as they stood that morning (before today's night was folded in).
    pub baselines: Vec<BaselineRow>,
    pub confidence: String,
    pub recovery: ScoreResult<RecoveryBreakdown>,
    pub sleep_performance: ScoreResult<SleepPerformanceBreakdown>,
    /// Need the main sleep was scored against.
    pub sleep_need: Option<SleepNeed>,
    /// Tonight's need and bedtimes (Peak / Perform / Get by).
    pub tonight: Option<SleepPlanner>,
    pub tonight_need: Option<SleepNeed>,
    pub strain: Option<StrainDetail>,
    pub stress_summary: StressSummary,
    pub stress_provisional: bool,
    pub health_monitor: HealthMonitor,
    pub baseline_set: BaselineSet,
    pub calibration: Calibration,
    /// Periods (>= 5 min) with no HR data between the first and last sample.
    pub gaps: Vec<DataGap>,
    /// Human-readable caveats (e.g. "no main sleep detected").
    pub notes: Vec<String>,
}

fn sorted_window<T: Copy>(v: &[T], a: i64, b: i64, ts: impl Fn(&T) -> i64) -> Vec<T> {
    let mut o: Vec<T> = v.iter().copied().filter(|x| ts(x) >= a && ts(x) < b).collect();
    o.sort_by_key(|x| ts(x));
    o
}

/// Periods of at least 5 minutes between consecutive HR samples.
pub fn find_gaps(hr: &[HrSample], min_gap_ms: i64) -> Vec<DataGap> {
    hr.windows(2).filter(|w| w[1].ts_ms - w[0].ts_ms >= min_gap_ms).map(|w| DataGap { start_ms: w[0].ts_ms, end_ms: w[1].ts_ms }).collect()
}

fn r1(x: f64) -> f64 {
    round1(x)
}

/// Compute everything for one local day. Pure and deterministic; robust to gaps (no data is invented:
/// missing inputs yield `None` / `Unavailable` / `Calibrating` and a note).
pub fn compute_day(inp: &DayInput) -> DayResult {
    let tz = inp.tz_offset_min;
    let day_idx = parse_day(inp.day).unwrap_or(0);
    let day_start = day_start_ms(day_idx, tz);
    let day_end = day_start + MS_PER_DAY;
    let search_start = day_start - 6 * 3_600_000;
    let s = &inp.streams;
    let hr = sorted_window(s.hr, search_start, day_end, |x| x.ts_ms);
    let rr = sorted_window(s.rr, search_start, day_end, |x| x.ts_ms);
    let motion = sorted_window(s.motion, search_start, day_end, |x| x.ts_ms);
    let temp = sorted_window(s.temp, search_start, day_end, |x| x.ts_ms);
    let spo2 = sorted_window(s.spo2, search_start, day_end, |x| x.ts_ms);
    let mut notes: Vec<String> = Vec::new();

    let mut prior: Vec<&DailySummary> = inp.history.iter().filter(|d| d.day.as_str() < inp.day).collect();
    prior.sort_by(|a, b| a.day.cmp(&b.day));
    let prev_day = format_day(day_idx - 1);
    let prev = prior.iter().find(|d| d.day == prev_day).copied();

    let gaps = find_gaps(&hr, 5 * MS_PER_MIN);
    let gap_total_min: i64 = gaps.iter().map(|g| (g.end_ms - g.start_ms) / MS_PER_MIN).sum();

    // ---- baselines (as of this morning) ----
    let bl = baselines_from_history(inp.history, inp.day);
    let nights = valid_nights(inp.history, inp.day);
    let calibration = calibration_state(nights);

    // ---- sleep ----
    let windows: Vec<SleepWindow> = detect_sleep(&hr, &motion, search_start, day_end).into_iter().filter(|w| w.end_ms >= day_start && w.end_ms < day_end).collect();
    let main_idx = windows
        .iter()
        .enumerate()
        .filter(|(_, w)| (w.end_ms - w.onset_ms) >= MIN_MAIN_SLEEP_MIN * MS_PER_MIN)
        .max_by_key(|(_, w)| w.end_ms - w.onset_ms)
        .map(|(i, _)| i);
    let mut sleeps: Vec<Sleep> = Vec::new();
    if let Some(i) = main_idx {
        sleeps.push(analyze_sleep(&windows[i], &hr, &rr, &motion, false));
    }
    for (i, w) in windows.iter().enumerate() {
        if Some(i) != main_idx {
            sleeps.push(analyze_sleep(w, &hr, &rr, &motion, true));
        }
    }
    let has_main = main_idx.is_some();
    if !has_main {
        notes.push("no main sleep detected for this day".into());
    }
    let nap_min: i64 = sleeps.iter().filter(|x| x.is_nap).map(|x| x.asleep_min).sum();
    let sleep_ranges: Vec<(i64, i64)> = sleeps.iter().map(|x| (x.start_ms, x.end_ms)).collect();

    // ---- overnight measurements ----
    let (main_hrv, main_rhr, main_resp, skin_temp_c, spo2_v) = match sleeps.first().filter(|x| !x.is_nap) {
        Some(m) => (m.hrv_ms, m.resting_hr, m.resp_rate, median_skin_temp(&temp, m.onset_ms, m.end_ms), spo2_median(&spo2, m.onset_ms, m.end_ms)),
        None => (None, None, None, None, None),
    };
    if has_main && main_hrv.is_none() {
        notes.push("no usable R-R data during sleep: HRV unavailable".into());
    }
    let skin_dev = skin_temp_c.zip(bl.skin_temp.as_ref()).map(|(v, b)| skin_temp_deviation(v, b.mean));

    // ---- day HR, zones, strain, activities, calories ----
    let hr_day: Vec<HrSample> = hr.iter().copied().filter(|x| x.ts_ms >= day_start).collect();
    let max = max_hr(inp.profile, &hr_day);
    let rhr_used = main_rhr
        .or(bl.resting_hr.map(|b| b.mean))
        .or_else(|| percentile(&hr_day.iter().map(|x| x.bpm).collect::<Vec<_>>(), 5.0))
        .unwrap_or(60.0);
    let zones = hr_zones(max, rhr_used.min(max - 20.0));
    let zone_min = time_in_zones(&hr_day, &zones, day_start, day_end);
    let load = training_load(&hr_day, &zones, day_start, day_end);
    let day_strain = (hr_day.len() >= 600).then(|| strain_from_load(load));
    if day_strain.is_none() {
        notes.push("fewer than 10 minutes of heart-rate data: strain unavailable".into());
    }
    let activities = detect_activities(&hr_day, &motion, &sleep_ranges, &zones, inp.profile, day_start, day_end);
    let kcal = (!hr_day.is_empty()).then(|| calories(&hr_day, inp.profile, &zones, day_start, day_end));
    let avg_hr = (!hr_day.is_empty()).then(|| hr_day.iter().map(|x| x.bpm).sum::<f64>() / hr_day.len() as f64);
    let steps = steps_total(s.steps, day_start, day_end);

    // ---- stress ----
    let full = stress_series(&hr, &rr, &motion, search_start, day_end, &sleep_ranges, (bl.stress_hr, bl.stress_hrv), (bl.resting_hr, bl.hrv));
    let stress: Vec<StressPoint> = full.points.iter().filter(|p| p.ts_ms > day_start).cloned().collect();
    let mut stress_summary = summarize_stress(&stress, &sleep_ranges);
    stress_summary.sleep_high_share = full.summary.sleep_high_share;
    if full.provisional {
        notes.push("stress uses today's own distribution (no 14-day baseline yet)".into());
    }

    // ---- sleep need / performance ----
    let asleep_hist: Vec<f64> = prior.iter().filter_map(|d| d.sleep_asleep_min.map(|a| a as f64)).collect();
    let base_need = baseline_sleep_need(&asleep_hist);
    let prev_strain = prev.and_then(|d| d.day_strain).unwrap_or(0.0);
    if prev.and_then(|d| d.day_strain).is_none() {
        notes.push("previous day's strain unknown: no strain extra added to sleep need".into());
    }
    let debt_in = debt_entering(inp.history, inp.day, base_need);
    let need_last = sleep_need(base_need, prev_strain, debt_in, prev.map_or(0.0, |d| d.nap_min as f64));
    let mut sp: ScoreResult<SleepPerformanceBreakdown> = ScoreResult::Unavailable { reason: "no main sleep detected".into() };
    let mut debt_out: Option<f64> = None;
    let mut main_sleep_info: Option<(i64, i64, i64, i64, f64)> = None;
    if let Some(m) = sleeps.first_mut().filter(|x| !x.is_nap) {
        let mut beds: Vec<f64> = prior.iter().rev().take(13).filter_map(|d| d.sleep_start_ms.map(|t| local_clock_min(t, tz))).collect();
        let mut wakes: Vec<f64> = prior.iter().rev().take(13).filter_map(|d| d.sleep_end_ms.map(|t| local_clock_min(t, tz))).collect();
        beds.push(local_clock_min(m.start_ms, tz));
        wakes.push(local_clock_min(m.end_ms, tz));
        let consistency_v = sleep_consistency(&beds, &wakes);
        let stress_score_v = stress_summary.sleep_high_share.map(sleep_stress_score);
        let b = sleep_performance(m.asleep_min as f64, need_last, consistency_v, m.efficiency_pct, stress_score_v);
        debt_out = Some(debt_after_night(need_last.total_min, m.asleep_min as f64));
        m.need_baseline_min = Some(need_last.baseline_min.round() as i64);
        m.need_strain_min = Some(need_last.strain_min.round() as i64);
        m.need_debt_min = Some(need_last.debt_min.round() as i64);
        m.need_naps_min = Some(need_last.naps_min.round() as i64);
        main_sleep_info = Some((m.start_ms, m.end_ms, m.asleep_min, m.in_bed_min, m.efficiency_pct));
        if b.reweighted {
            notes.push("sleep performance re-weighted: a contributor was unavailable".into());
        }
        sp = ScoreResult::Score(b);
    }

    // ---- recovery & strain target ----
    let rec_inputs = RecoveryInputs {
        hrv_ms: main_hrv,
        resting_hr: main_rhr,
        resp_rate: main_resp,
        sleep_performance: sp.score().map(|b| b.score),
        skin_temp_c,
    };
    let recovery = if has_main {
        recovery(&rec_inputs, &bl, nights)
    } else if nights < MIN_NIGHTS {
        ScoreResult::Calibrating { nights_remaining: MIN_NIGHTS - nights }
    } else {
        ScoreResult::Unavailable { reason: "no main sleep detected".into() }
    };
    let rec_band = recovery.score().map(|r| r.band);
    let target = rec_band.map(strain_target);
    let strain_detail = day_strain.map(|ds| StrainDetail {
        day_strain: ds,
        band: strain_band(ds),
        load,
        target,
        zone_minutes: zone_min,
        max_hr: max,
        resting_hr_used: rhr_used,
        zones,
    });

    // ---- health monitor ----
    let monitor = health_monitor(main_resp, spo2_v, main_rhr, main_hrv, skin_temp_c, &bl, nights);

    // ---- tonight's planner ----
    let (tonight, tonight_need) = {
        let carry = debt_out.unwrap_or(debt_in);
        let need = sleep_need(base_need, day_strain.unwrap_or(0.0), carry, nap_min as f64);
        let mut wakes: Vec<f64> = prior.iter().rev().take(13).filter_map(|d| d.sleep_end_ms.map(|t| local_clock_min(t, tz))).collect();
        if let Some((_, e, ..)) = main_sleep_info {
            wakes.push(local_clock_min(e, tz));
        }
        let wake_clock = circular_mean_min(&wakes).unwrap_or(420.0);
        let mut effs: Vec<f64> = prior.iter().rev().take(13).filter_map(|d| d.sleep_efficiency).collect();
        if let Some((.., eff)) = main_sleep_info {
            effs.push(eff);
        }
        let eff = mean(&effs).map_or(0.88, |e| e / 100.0);
        let wake_ms = day_start + MS_PER_DAY + (wake_clock * MS_PER_MIN as f64).round() as i64;
        (Some(sleep_planner(need.total_min, wake_ms, eff, tz)), Some(need))
    };

    // ---- summary ----
    let (hours, cons, eff_c, stress_c) = match sp.score() {
        Some(b) => (Some(b.hours_vs_needed), b.consistency, Some(b.efficiency), b.sleep_stress),
        None => (None, None, None, None),
    };
    let summary = DailySummary {
        day: inp.day.to_string(),
        recovery: recovery.score().map(|r| r1(r.score)),
        recovery_band: rec_band.map(|b| b.as_str().to_string()),
        sleep_performance: sp.score().map(|b| r1(b.score)),
        hours_vs_needed: hours.map(r1),
        sleep_consistency: cons.map(r1),
        sleep_efficiency: eff_c.map(r1),
        sleep_stress: stress_c.map(r1),
        day_strain: day_strain.map(|x| (x * 100.0).round() / 100.0),
        strain_target_lo: target.map(|t| t.lo),
        strain_target_hi: target.map(|t| t.hi),
        hrv_ms: main_hrv.map(r1),
        resting_hr: main_rhr.map(r1),
        resp_rate: main_resp.map(r1),
        skin_temp_dev_c: skin_dev.map(|x| (x * 100.0).round() / 100.0),
        spo2: spo2_v.map(r1),
        calories: kcal.map(|k| k.round()),
        steps,
        avg_hr: avg_hr.map(r1),
        zone1_min: zone_min[0].round() as i64,
        zone2_min: zone_min[1].round() as i64,
        zone3_min: zone_min[2].round() as i64,
        zone4_min: zone_min[3].round() as i64,
        zone5_min: zone_min[4].round() as i64,
        confidence: String::new(),
        algo_version: ALGO_VERSION,
        skin_temp_c,
        stress_ref_hr: full.ref_hr,
        stress_ref_hrv: full.ref_hrv,
        sleep_start_ms: main_sleep_info.map(|m| m.0),
        sleep_end_ms: main_sleep_info.map(|m| m.1),
        sleep_asleep_min: main_sleep_info.map(|m| m.2),
        sleep_in_bed_min: main_sleep_info.map(|m| m.3),
        nap_min,
        sleep_need_min: has_main.then(|| need_last.total_min.round() as i64),
        sleep_debt_out_min: debt_out.map(|d| d.round() as i64),
        calibration_nights_remaining: match recovery {
            ScoreResult::Calibrating { nights_remaining } => Some(nights_remaining),
            _ => None,
        },
    };
    let tier = match calibration {
        Calibration::Calibrating { .. } => "calibrating",
        Calibration::Provisional { .. } => "building",
        Calibration::Solid { .. } => "solid",
    };
    let mut flags: Vec<String> = Vec::new();
    if !gaps.is_empty() {
        flags.push(format!("{} data gap(s), {} min", gaps.len(), gap_total_min));
    }
    if !has_main {
        flags.push("no main sleep".into());
    }
    if has_main && main_hrv.is_none() {
        flags.push("no HRV".into());
    }
    if full.provisional {
        flags.push("stress provisional".into());
    }
    let confidence = if flags.is_empty() { tier.to_string() } else { format!("{tier}; {}", flags.join(", ")) };
    let mut summary = summary;
    summary.confidence = confidence.clone();

    DayResult {
        day: inp.day.to_string(),
        summary,
        sleeps,
        activities,
        stress,
        baselines: baseline_rows(inp.day, &bl),
        confidence,
        recovery,
        sleep_performance: sp,
        sleep_need: has_main.then_some(need_last),
        tonight,
        tonight_need,
        strain: strain_detail,
        stress_summary,
        stress_provisional: full.provisional,
        health_monitor: monitor,
        baseline_set: bl,
        calibration,
        gaps,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::super::signals::testgen::*;
    use super::super::sleep::testgen_pub::synthetic_night;
    use super::*;

    pub(super) const DAY: &str = "2026-03-10";
    const H: i64 = 3_600_000;

    pub(super) struct Synth {
        pub hr: Vec<HrSample>,
        pub rr: Vec<RrInterval>,
        pub motion: Vec<MotionSample>,
        pub temp: Vec<TempSample>,
        pub day_start: i64,
    }

    /// 30 h of samples from 18:00 the evening before to midnight after `DAY` (UTC = local):
    /// evening awake; 22:30 lying; 23:00-06:00 sleep with cycles (from `synthetic_night`); day at desk;
    /// workout 17:00-17:45 (HR ~150, motion 0.4); evening awake.
    pub(super) fn synth_day() -> Synth {
        let day_start = days_from_civil(2026, 3, 10) * MS_PER_DAY;
        let t0 = day_start - 6 * H; // 18:00 previous evening
        let mut rng = Rng(99);
        let (mut hr, mut rr, mut motion, mut temp) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let night_start = t0 + 4 * H + 30 * MS_PER_MIN; // 22:30 (lying starts)
        let (nhr, nrr, nmo, w, _) = synthetic_night();
        let night_end = night_start + w.end_ms; // 06:00
        let wake_seg = |hr: &mut Vec<HrSample>, rr: &mut Vec<RrInterval>, mo: &mut Vec<MotionSample>, a: i64, b: i64, bpm: f64, g: f64, rng: &mut Rng| {
            let mut t = a;
            while t < b {
                hr.push(HrSample { ts_ms: t, bpm: bpm + 2.0 * rng.gauss() });
                mo.push(MotionSample { ts_ms: t, magnitude_g: g * (0.5 + rng.next()) });
                t += 1000;
            }
            rr.extend(rsa_series(a, (b - a) / 1000, bpm, 15.0, 12.0, 10.0, rng));
        };
        wake_seg(&mut hr, &mut rr, &mut motion, t0, night_start, 68.0, 0.03, &mut rng);
        hr.extend(nhr.iter().map(|s| HrSample { ts_ms: s.ts_ms + night_start, bpm: s.bpm }));
        rr.extend(nrr.iter().map(|s| RrInterval { ts_ms: s.ts_ms + night_start, rr_ms: s.rr_ms }));
        motion.extend(nmo.iter().map(|s| MotionSample { ts_ms: s.ts_ms + night_start, magnitude_g: s.magnitude_g }));
        wake_seg(&mut hr, &mut rr, &mut motion, night_end, day_start + 17 * H, 70.0, 0.03, &mut rng);
        // workout: 5 min ramp 100->150 then 40 min ~150
        let ws = day_start + 17 * H;
        let mut t = ws;
        while t < ws + 45 * MS_PER_MIN {
            let el = (t - ws) as f64 / MS_PER_MIN as f64;
            let bpm = if el < 5.0 { 100.0 + 10.0 * el } else { 150.0 };
            hr.push(HrSample { ts_ms: t, bpm: bpm + 3.0 * rng.gauss() });
            motion.push(MotionSample { ts_ms: t, magnitude_g: 0.4 * (0.5 + rng.next()) });
            t += 1000;
        }
        rr.extend(rsa_series(ws, 45 * 60, 150.0, 25.0, 5.0, 5.0, &mut rng));
        wake_seg(&mut hr, &mut rr, &mut motion, ws + 45 * MS_PER_MIN, day_start + 24 * H, 72.0, 0.03, &mut rng);
        let mut t = t0;
        while t < day_start + 24 * H {
            let asleep = t >= night_start + 30 * MS_PER_MIN && t < night_end;
            temp.push(TempSample { ts_ms: t, celsius: if asleep { 33.6 } else { 33.0 } + 0.02 * rng.gauss() });
            t += 60_000;
        }
        Synth { hr, rr, motion, temp, day_start }
    }

    pub(super) fn history(n: usize) -> Vec<DailySummary> {
        (0..n)
            .map(|i| {
                let d = parse_day(DAY).unwrap() - (n - i) as i64;
                let bed = d * MS_PER_DAY - 90 * MS_PER_MIN + (i as i64 % 3 - 1) * 10 * MS_PER_MIN; // ~22:30
                DailySummary {
                    day: format_day(d),
                    hrv_ms: Some(50.0 + (i % 3) as f64 * 2.0),
                    resting_hr: Some(49.0 + (i % 2) as f64),
                    resp_rate: Some(14.5 + (i % 3) as f64 * 0.2),
                    skin_temp_c: Some(33.5 + (i % 2) as f64 * 0.1),
                    stress_ref_hr: Some(70.0 + (i % 2) as f64),
                    stress_ref_hrv: Some(21.0 + (i % 3) as f64),
                    day_strain: Some(10.0),
                    sleep_start_ms: Some(bed),
                    sleep_end_ms: Some(bed + 450 * MS_PER_MIN),
                    sleep_asleep_min: Some(420),
                    sleep_in_bed_min: Some(450),
                    sleep_efficiency: Some(92.0),
                    ..Default::default()
                }
            })
            .collect()
    }

    fn run(sy: &Synth, hist: &[DailySummary]) -> DayResult {
        let p = UserProfile { age: 30.0, sex: Sex::Male, weight_kg: 75.0, height_cm: Some(178.0), max_hr_override: None };
        let input = DayInput {
            day: DAY,
            tz_offset_min: 0,
            profile: &p,
            streams: RawStreams { hr: &sy.hr, rr: &sy.rr, motion: &sy.motion, temp: &sy.temp, ..Default::default() },
            history: hist,
        };
        compute_day(&input)
    }

    #[test]
    fn end_to_end_24h() {
        let sy = synth_day();
        let hist = history(14);
        let r = run(&sy, &hist);
        let h = |hh: i64, mm: i64| sy.day_start + hh * H + mm * MS_PER_MIN;

        // main sleep: in-bed ~22:30 prev day (-1.5 h), wake ~06:00
        assert_eq!(r.sleeps.len(), 1, "{:?}", r.sleeps.iter().map(|s| (s.start_ms - sy.day_start, s.end_ms - sy.day_start)).collect::<Vec<_>>());
        let m = &r.sleeps[0];
        assert!(!m.is_nap);
        assert!((m.start_ms - h(-1, -30)).abs() < 20 * MS_PER_MIN, "bed {}", (m.start_ms - sy.day_start) / MS_PER_MIN);
        assert!((m.end_ms - h(6, 0)).abs() < 15 * MS_PER_MIN, "wake {}", (m.end_ms - sy.day_start) / MS_PER_MIN);
        assert!(m.asleep_min > 360 && m.asleep_min < 460, "asleep {}", m.asleep_min);
        assert!(m.efficiency_pct > 85.0);
        assert!(m.deep_min > 20 && m.rem_min > 20 && m.light_min > 100);
        assert_eq!(r.summary.sleep_asleep_min, Some(m.asleep_min));

        // overnight vitals
        let rhr = r.summary.resting_hr.unwrap();
        assert!((44.0..=54.0).contains(&rhr), "rhr {rhr}");
        let hrv = r.summary.hrv_ms.unwrap();
        assert!((40.0..=100.0).contains(&hrv), "hrv {hrv}");
        assert!((r.summary.resp_rate.unwrap() - 14.5).abs() < 2.0);
        // skin temp 33.6 vs baseline ~33.55 -> small deviation
        assert!(r.summary.skin_temp_dev_c.unwrap().abs() < 0.3);
        assert!(r.summary.spo2.is_none()); // no SpO2 stream supplied

        // recovery with 14 solid nights
        assert_eq!(r.calibration, Calibration::Solid { nights: 14 });
        let rec = r.recovery.score().expect("recovery");
        assert!((0.0..=100.0).contains(&rec.score));
        assert_eq!(r.summary.recovery_band.as_deref(), Some(rec.band.as_str()));
        assert!(rec.score > 55.0, "HRV is above baseline so recovery should be decent: {}", rec.score);
        assert_eq!(rec.contributors.len(), 5);
        assert!(r.confidence.starts_with("solid"), "{}", r.confidence);

        // sleep performance & need
        let sp = r.sleep_performance.score().expect("sleep performance");
        assert!(sp.score > 60.0 && sp.score <= 100.0, "{}", sp.score);
        assert!(sp.consistency.unwrap() > 85.0);
        let need = r.sleep_need.unwrap();
        assert_eq!(need.strain_min, 0.0); // previous day strain 10 -> no extra
        assert!(need.baseline_min >= 390.0 && need.baseline_min <= 540.0);
        assert_eq!(m.need_baseline_min, Some(need.baseline_min.round() as i64));

        // workout detected 17:00-17:45
        assert_eq!(r.activities.len(), 1, "{:?}", r.activities);
        let a = &r.activities[0];
        assert!((a.start_ms - h(17, 0)).abs() <= 6 * MS_PER_MIN);
        assert!((a.end_ms - h(17, 45)).abs() <= 3 * MS_PER_MIN);
        assert!(a.avg_hr > 135.0 && a.max_hr > 150.0 && a.calories > 250.0);
        assert!(a.strain > 8.0 && a.strain < 14.0, "activity strain {}", a.strain);

        // day strain in a plausible band and consistent with the activity
        let st = r.strain.as_ref().unwrap();
        assert!(st.day_strain >= a.strain - 0.01 && st.day_strain < 15.0, "{}", st.day_strain);
        assert!(matches!(st.band, StrainBand::Moderate | StrainBand::High));
        let tgt = st.target.unwrap();
        let expect = strain_target(rec.band);
        assert_eq!((tgt.lo, tgt.hi), (expect.lo, expect.hi));
        assert_eq!(r.summary.strain_target_lo, Some(expect.lo));
        assert!(r.summary.zone3_min >= 15, "zone3 {}", r.summary.zone3_min);
        assert!(r.summary.calories.unwrap() > 1800.0);
        assert!(r.summary.steps.is_none());

        // stress: calm most of the day, none during the workout
        assert!(r.stress.len() > 100, "{}", r.stress.len());
        assert!(r.stress.iter().all(|p| p.ts_ms <= h(17, 0) || p.ts_ms >= h(17, 45) + 5 * MS_PER_MIN));
        assert!(r.stress.iter().all(|p| (0.0..=3.0).contains(&p.score)));
        let s = &r.stress_summary;
        assert!(s.low_min > s.high_min);
        assert!(s.sleep_high_share.is_some());

        // health monitor reads in range, no early warning
        assert!(!r.health_monitor.early_warning);
        assert!(r.health_monitor.in_range_count >= 3, "{:?}", r.health_monitor);

        // tonight
        let t = r.tonight.as_ref().unwrap();
        assert_eq!(t.options.len(), 3);
        assert!(t.options[0].bed_ms < t.options[1].bed_ms && t.options[1].bed_ms < t.options[2].bed_ms);
        assert!(t.options[0].bed_ms > h(24, 0) - 14 * H && t.options[0].bed_ms < h(24, 0) + 2 * H);

        // storage rows
        assert!(!r.baselines.is_empty() && r.baselines.iter().all(|b| b.day == DAY));
        assert!(r.gaps.is_empty());
        assert_eq!(r.summary.algo_version, ALGO_VERSION);
        // result serialises (the app sends it to the UI)
        let _ = format!("{:?}", r.summary);
    }

    #[test]
    fn calibrating_with_no_history_gives_no_recovery() {
        let sy = synth_day();
        let r = run(&sy, &[]);
        assert_eq!(r.recovery, ScoreResult::Calibrating { nights_remaining: 4 });
        assert_eq!(r.summary.recovery, None);
        assert_eq!(r.summary.calibration_nights_remaining, Some(4));
        assert!(r.confidence.starts_with("calibrating"));
        assert!(r.sleep_performance.score().is_some()); // sleep itself still scored
        assert!(r.summary.day_strain.is_some());
        assert!(r.strain.unwrap().target.is_none()); // no recovery band -> no target
        assert!(r.stress_provisional);
        // 3 prior nights: 1 more to go
        assert_eq!(run(&sy, &history(3)).recovery, ScoreResult::Calibrating { nights_remaining: 1 });
        assert!(run(&sy, &history(4)).recovery.score().is_some());
    }

    #[test]
    fn strap_off_overnight_reports_gap_not_data() {
        let mut sy = synth_day();
        // strap removed 02:00-03:30
        let (a, b) = (sy.day_start + 2 * H, sy.day_start + 3 * H + 30 * MS_PER_MIN);
        sy.hr.retain(|s| s.ts_ms < a || s.ts_ms >= b);
        sy.rr.retain(|s| s.ts_ms < a || s.ts_ms >= b);
        sy.motion.retain(|s| s.ts_ms < a || s.ts_ms >= b);
        let r = run(&sy, &history(14));
        assert_eq!(r.gaps.len(), 1);
        assert!((r.gaps[0].start_ms - a).abs() < 5_000 && (r.gaps[0].end_ms - b).abs() < 5_000);
        assert!(r.confidence.contains("gap"), "{}", r.confidence);
        // the sleep survives the gap (merged), but the missing 90 min are neither asleep nor awake
        let m = r.sleeps.iter().find(|s| !s.is_nap).unwrap();
        assert!(m.gap_min >= 80, "gap_min {}", m.gap_min);
        assert!(m.asleep_min < 7 * 60 - 60, "asleep {}", m.asleep_min);
        assert!(m.stages.iter().all(|s| s.end_ms <= a || s.start_ms >= b - 30_000));
    }

    #[test]
    fn no_data_at_all_is_unavailable_not_zero() {
        let p = UserProfile::default();
        let input = DayInput { day: DAY, tz_offset_min: 0, profile: &p, streams: RawStreams::default(), history: &[] };
        let r = compute_day(&input);
        assert!(r.sleeps.is_empty() && r.activities.is_empty() && r.stress.is_empty());
        assert!(matches!(r.recovery, ScoreResult::Calibrating { .. }));
        assert!(matches!(r.sleep_performance, ScoreResult::Unavailable { .. }));
        assert_eq!(r.summary.day_strain, None);
        assert_eq!(r.summary.calories, None);
        assert!(!r.notes.is_empty());
        let r2 = compute_day(&DayInput { history: &history(14), ..input });
        assert!(matches!(r2.recovery, ScoreResult::Unavailable { .. }));
    }

    #[test]
    fn daytime_nap_is_separate_and_reduces_tonights_need() {
        let mut sy = synth_day();
        // 13:00-13:40 nap: HR 52, still, replacing the desk samples
        let (a, b) = (sy.day_start + 13 * H, sy.day_start + 13 * H + 40 * MS_PER_MIN);
        let mut rng = Rng(3);
        for s in sy.hr.iter_mut().filter(|s| s.ts_ms >= a && s.ts_ms < b) {
            s.bpm = 50.0 + rng.gauss() * 0.5;
        }
        for s in sy.motion.iter_mut().filter(|s| s.ts_ms >= a && s.ts_ms < b) {
            s.magnitude_g = 0.003;
        }
        let r = run(&sy, &history(14));
        let naps: Vec<_> = r.sleeps.iter().filter(|s| s.is_nap).collect();
        assert_eq!(naps.len(), 1, "{:?}", r.sleeps.iter().map(|s| (s.is_nap, (s.start_ms - sy.day_start) / MS_PER_MIN)).collect::<Vec<_>>());
        assert!(!r.sleeps[0].is_nap);
        assert!(r.summary.nap_min >= 20);
        let tn = r.tonight_need.unwrap();
        assert!(tn.naps_min >= 20.0);
        // nap hours are excluded from the activity detector and do not become an activity
        assert_eq!(r.activities.len(), 1);
    }
}
