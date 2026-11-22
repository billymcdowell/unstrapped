//! Score computation: after a sync / rebuild, recompute the affected local days in chronological order
//! with `unstrapped_core::metrics`, store the results, and generate weekly / monthly assessments.

use crate::db::Db;
use crate::emitter::{Emitter, emit};
use crate::model::*;
use chrono::{Local, TimeZone};
use unstrapped_core::metrics::hr::{hr_zones, time_in_zones, calories as calories_fn, tanaka_max_hr};
use unstrapped_core::metrics::strain::strain_for_window;
use unstrapped_core::metrics::util::*;
use unstrapped_core::metrics::{DayInput, HrSample, RawStreams, UserProfile, compute_day};

#[derive(Debug, Default)]
pub struct ComputeOutcome {
    pub days: Vec<String>,
    pub new_assessments: Vec<AssessmentListItem>,
}

/// Local UTC offset (minutes east) in effect at `ts_ms`.
pub fn tz_offset_at(ts_ms: i64) -> i32 {
    Local.timestamp_millis_opt(ts_ms).single().map(|d| d.offset().local_minus_utc() / 60).unwrap_or(0)
}

/// Offset for a local day (taken at noon UTC of that date, close enough for DST edges).
pub fn tz_offset_for_day(day: &str) -> i32 {
    parse_day(day).map(|i| tz_offset_at(i * MS_PER_DAY + 12 * 3_600_000)).unwrap_or(0)
}

pub fn today_string() -> String {
    let now = chrono::Utc::now().timestamp_millis();
    local_day_string(now, tz_offset_at(now))
}

pub fn day_of(ts_ms: i64) -> String {
    local_day_string(ts_ms, tz_offset_at(ts_ms))
}

/// `[start, end)` of a local day in Unix ms.
pub fn day_bounds(day: &str) -> Option<(i64, i64)> {
    let idx = parse_day(day)?;
    let s = day_start_ms(idx, tz_offset_for_day(day));
    Some((s, s + MS_PER_DAY))
}

pub fn days_range(from: &str, to: &str) -> Vec<String> {
    match (parse_day(from), parse_day(to)) {
        (Some(a), Some(b)) if b >= a => (a..=b).map(format_day).collect(),
        _ => vec![],
    }
}

pub fn add_days(day: &str, n: i64) -> String {
    format_day(parse_day(day).unwrap_or(0) + n)
}

fn progress(em: &dyn Emitter, i: usize, n: usize) {
    let f = if n == 0 { 0.0 } else { i as f64 / n as f64 };
    emit(em, "sync://progress", &SyncProgress::new("processing", 0.5 + 0.49 * f, format!("Calculating scores… day {} of {}", i.min(n), n)));
}

/// Recompute `days` (chronological order) and store them.
pub fn recompute_days(db: &Db, em: Option<&dyn Emitter>, days: &[String]) -> Result<(), String> {
    let settings = db.settings();
    let profile = settings.user_profile();
    for (i, day) in days.iter().enumerate() {
        if let Some(em) = em {
            progress(em, i + 1, days.len());
        }
        recompute_day(db, &profile, day)?;
    }
    Ok(())
}

pub fn recompute_day(db: &Db, profile: &UserProfile, day: &str) -> Result<(), String> {
    let (start, end) = day_bounds(day).ok_or_else(|| format!("bad day '{day}'"))?;
    let tz = tz_offset_for_day(day);
    let streams = db.load_streams(start - 6 * 3_600_000, end)?;
    let history = db.summaries_before(day, 100)?;
    let input = DayInput {
        day,
        tz_offset_min: tz,
        profile,
        streams: RawStreams { hr: &streams.hr, rr: &streams.rr, motion: &streams.motion, temp: &streams.temp, spo2: &streams.spo2, steps: &streams.steps },
        history: &history,
    };
    let res = compute_day(&input);
    db.write_day_result(&res, start, end)?;
    // manual activities are re-measured against the (possibly new) heart-rate data
    for a in db.manual_activities_for_day(day)? {
        let zones = res.strain.as_ref().map(|s| s.zones).unwrap_or_else(|| hr_zones(tanaka_max_hr(profile.age), 60.0));
        let hr: Vec<HrSample> = streams.hr.iter().copied().filter(|h| h.ts_ms >= a.start_ms && h.ts_ms < a.end_ms).collect();
        let m = measure_window(&hr, profile, &zones, a.start_ms, a.end_ms);
        db.upsert_manual_activity(day, &ActivitySummary { strain: m.0, avg_hr: m.1, max_hr: m.2, calories: m.3, zone_min: m.4, ..a })?;
    }
    Ok(())
}

/// (strain, avg hr, max hr, kcal, zone minutes) of a manual window.
pub fn measure_window(hr: &[HrSample], profile: &UserProfile, zones: &unstrapped_core::metrics::hr::HrZones, start: i64, end: i64) -> (f64, f64, f64, f64, [f64; 5]) {
    if hr.is_empty() {
        return (0.0, 0.0, 0.0, 0.0, [0.0; 5]);
    }
    let strain = strain_for_window(hr, zones, start, end);
    let avg = hr.iter().map(|h| h.bpm).sum::<f64>() / hr.len() as f64;
    let max = hr.iter().map(|h| h.bpm).fold(0.0, f64::max);
    let kcal = calories_fn(hr, profile, zones, start, end);
    let zm = time_in_zones(hr, zones, start, end);
    (round2(strain), round1(avg), max, kcal.round(), zm.map(|m| m.round()))
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// After a sync: recompute the days touched by `range` (plus days that depend on them), then assessments.
pub fn process_new_data(db: &Db, em: &dyn Emitter, range: Option<(i64, i64)>) -> Result<ComputeOutcome, String> {
    let today = today_string();
    let mut days: Vec<String> = vec![];
    if let Some((a, b)) = range {
        let first = day_of(a);
        let mut last = add_days(&day_of(b), 1); // evening data feeds the next morning's sleep
        if last > today {
            last = today.clone();
        }
        if let Some(lc) = db.last_computed_day() {
            let lc = if lc > today { today.clone() } else { lc };
            if lc > last {
                last = lc;
            }
        }
        days = days_range(&first, &last);
    }
    if !days.is_empty() {
        recompute_days(db, Some(em), &days)?;
    }
    let new_assessments = crate::assess::generate_due(db, &today);
    Ok(ComputeOutcome { days, new_assessments })
}

/// `rebuild_decoded`: re-decode raw frames, then recompute everything.
pub fn rebuild_all(db: &Db, em: &dyn Emitter) -> Result<Vec<String>, String> {
    db.rebuild_decoded()?;
    db.clear_computed()?;
    let Some((min, _max)) = db.data_bounds() else { return Ok(vec![]) };
    let today = today_string();
    let days = days_range(&day_of(min), &today);
    recompute_days(db, Some(em), &days)?;
    crate::assess::generate_due(db, &today);
    Ok(days)
}

/// Recompute every day that has data (e.g. after a profile change) without re-decoding.
pub fn rescore_all(db: &Db, em: &dyn Emitter) -> Result<Vec<String>, String> {
    let Some((min, _)) = db.data_bounds() else { return Ok(vec![]) };
    let days = days_range(&day_of(min), &today_string());
    recompute_days(db, Some(em), &days)?;
    Ok(days)
}
