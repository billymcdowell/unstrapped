//! Trends, journal, insights, healthspan, raw dashboard data and CSV export.

use crate::compute::{add_days, day_bounds, day_of, days_range, today_string};
use crate::db::Db;
use crate::model::*;
use crate::screens::{minute_series, band_str};
use std::collections::BTreeMap;
use unstrapped_core::metrics::healthspan::{HealthspanInputs, estimate_vo2max, healthspan};
use unstrapped_core::metrics::insights::{DayOutcome, InsightStatus, behavior_insights};
use unstrapped_core::metrics::scores::recovery_band;
use unstrapped_core::metrics::trends::{DayValue, period_summary, weekly_averages};
use unstrapped_core::metrics::DailySummary;

type R<T> = Result<T, String>;

fn r1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}
fn r2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

pub fn range_days(range: &str) -> R<i64> {
    Ok(match range {
        "1W" => 7,
        "1M" => 30,
        "6M" => 182,
        "1Y" => 365,
        other => return Err(format!("Unknown range '{other}'")),
    })
}

pub struct TrendMeta {
    pub label: &'static str,
    pub unit: &'static str,
    pub higher_is_better: bool,
}

pub fn trend_meta(metric: &str) -> Option<TrendMeta> {
    let (label, unit, hib) = match metric {
        "sleepPerformance" => ("Sleep Performance", "%", true),
        "hoursVsNeed" => ("Hours vs. Need", "%", true),
        "timeInBed" => ("Time in Bed", "h", true),
        "consistency" => ("Sleep Consistency", "%", true),
        "restorativePct" => ("Restorative Sleep", "%", true),
        "restorativeHours" => ("Restorative Sleep", "h", true),
        "efficiency" => ("Sleep Efficiency", "%", true),
        "sleepDebt" => ("Sleep Debt", "min", false),
        "dayStrain" => ("Day Strain", "", true),
        "strengthTime" => ("Strength Activity Time", "min", true),
        "calories" => ("Calories", "kcal", true),
        "avgHr" => ("Average HR", "bpm", true),
        "steps" => ("Steps", "", true),
        "zones123" => ("Zones 1-3", "min", true),
        "zones45" => ("Zones 4-5", "min", true),
        "vo2max" => ("VO2 Max", "ml/kg/min", true),
        "recovery" => ("Recovery", "%", true),
        "restingHr" => ("Resting HR", "bpm", false),
        "hrv" => ("HRV", "ms", true),
        "respRate" => ("Respiratory Rate", "rpm", false),
        "stress" => ("Stress", "", false),
        _ => return None,
    };
    Some(TrendMeta { label, unit, higher_is_better: hib })
}

/// Daily series of one trend metric over `[from, to]`.
pub fn daily_series(db: &Db, metric: &str, from: &str, to: &str) -> R<Vec<DayValue>> {
    let sums = db.summaries_between(from, to)?;
    let profile = db.settings().user_profile();
    let max_hr = unstrapped_core::metrics::hr::max_hr(&profile, &[]);
    let mut extra: BTreeMap<String, f64> = BTreeMap::new();
    match metric {
        "restorativePct" | "restorativeHours" => {
            let conn = db.conn();
            let mut q = conn.prepare("SELECT day, restorative_pct, deep_min + rem_min FROM sleeps WHERE is_nap = 0 AND day >= ?1 AND day <= ?2").map_err(|e| e.to_string())?;
            let rows = q.query_map([from, to], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?, r.get::<_, f64>(2)?))).map_err(|e| e.to_string())?;
            for row in rows.flatten() {
                extra.insert(row.0, if metric == "restorativePct" { r1(row.1) } else { r1(row.2 / 60.0) });
            }
        }
        "strengthTime" => {
            for (d, a) in db.activities_between(from, to)? {
                if a.kind.to_lowercase().contains("strength") {
                    *extra.entry(d).or_insert(0.0) += (a.end_ms - a.start_ms) as f64 / 60_000.0;
                }
            }
        }
        "stress" => {
            let (s, _) = day_bounds(from).ok_or("bad day")?;
            let (_, e) = day_bounds(to).ok_or("bad day")?;
            let mut acc: BTreeMap<String, (f64, usize)> = BTreeMap::new();
            for (t, v) in db.stress_range(s, e)? {
                let x = acc.entry(day_of(t)).or_insert((0.0, 0));
                x.0 += v;
                x.1 += 1;
            }
            for (d, (sum, n)) in acc {
                extra.insert(d, r2(sum / n as f64));
            }
        }
        _ => {}
    }
    Ok(days_range(from, to)
        .into_iter()
        .map(|d| {
            let s: Option<&DailySummary> = sums.iter().find(|s| s.day == d);
            let v: Option<f64> = match metric {
                "restorativePct" | "restorativeHours" | "strengthTime" | "stress" => {
                    if metric == "strengthTime" { s.map(|_| extra.get(&d).copied().unwrap_or(0.0)) } else { extra.get(&d).copied() }
                }
                _ => s.and_then(|s| match metric {
                    "sleepPerformance" => s.sleep_performance,
                    "hoursVsNeed" => s.hours_vs_needed,
                    "timeInBed" => s.sleep_in_bed_min.map(|m| r1(m as f64 / 60.0)),
                    "consistency" => s.sleep_consistency,
                    "efficiency" => s.sleep_efficiency,
                    "sleepDebt" => s.sleep_debt_out_min.map(|m| m as f64),
                    "dayStrain" => s.day_strain,
                    "calories" => s.calories,
                    "avgHr" => s.avg_hr,
                    "steps" => s.steps.map(|v| v as f64),
                    "zones123" => Some((s.zone1_min + s.zone2_min + s.zone3_min) as f64),
                    "zones45" => Some((s.zone4_min + s.zone5_min) as f64),
                    "vo2max" => s.resting_hr.map(|r| r1(estimate_vo2max(max_hr, r))),
                    "recovery" => s.recovery,
                    "restingHr" => s.resting_hr,
                    "hrv" => s.hrv_ms,
                    "respRate" => s.resp_rate,
                    _ => None,
                }),
            };
            DayValue { day: d, value: v }
        })
        .collect())
}

fn baseline_band(db: &Db, metric: &str) -> Option<BaselineBand> {
    let key = match metric {
        "restingHr" => "resting_hr",
        "hrv" => "hrv",
        "respRate" => "resp",
        _ => return None,
    };
    db.conn().query_row("SELECT lo, hi FROM baselines WHERE metric = ?1 ORDER BY day DESC LIMIT 1", [key], |r| Ok(BaselineBand { lo: r1(r.get::<_, f64>(0)?), hi: r1(r.get::<_, f64>(1)?) })).ok()
}

pub fn get_trend(db: &Db, metric: &str, range: &str) -> R<Avail<TrendData>> {
    let meta = trend_meta(metric).ok_or_else(|| format!("Unknown metric '{metric}'"))?;
    let days = range_days(range)?;
    let to = today_string();
    let from = add_days(&to, -(days - 1));
    let series = daily_series(db, metric, &from, &to)?;
    if series.iter().all(|d| d.value.is_none()) {
        return Ok(Avail::no_data("No data in this range yet."));
    }
    let weekly = matches!(range, "6M" | "1Y");
    let band_of = |v: f64| (metric == "recovery").then(|| band_str(recovery_band(v)));
    let points: Vec<TrendPoint> = if weekly {
        weekly_averages(&series).into_iter().map(|w| TrendPoint { band: band_of(w.avg), day: w.week_start, value: Some(r1(w.avg)) }).collect()
    } else {
        series.iter().map(|d| TrendPoint { day: d.day.clone(), value: d.value.map(r2), band: d.value.and_then(band_of) }).collect()
    };
    let summary = period_summary(&series, &to, days, meta.higher_is_better).and_then(|p| {
        Some(TrendSummary {
            avg: r1(p.avg?),
            change_pct: p.change_pct.map(r1),
            best: p.best.and_then(|b| Some(DayVal { day: b.day, value: r2(b.value?) }))?,
            worst: p.worst.and_then(|b| Some(DayVal { day: b.day, value: r2(b.value?) }))?,
        })
    });
    Ok(Avail::ok(TrendData {
        metric: metric.into(),
        label: meta.label.into(),
        unit: meta.unit.into(),
        range: range.into(),
        granularity: if weekly { "weekly" } else { "daily" }.into(),
        points,
        baseline: baseline_band(db, metric),
        summary,
    }))
}

// ============================================================================ journal
pub fn get_journal(db: &Db, day: &str) -> JournalDay {
    let answers = db.journal_answers(day);
    let questions = db
        .journal_selected()
        .into_iter()
        .filter_map(|id| {
            let item = crate::journal::find(&id)?;
            Some(JournalQuestion { answer: answers.iter().find(|a| a.0 == id).map(|a| a.1), id, label: item.label.into(), category: item.category.into() })
        })
        .collect();
    JournalDay { day: day.into(), questions }
}

pub fn get_journal_library(db: &Db) -> Vec<JournalLibraryItem> {
    let sel = db.journal_selected();
    crate::journal::LIBRARY
        .iter()
        .map(|i| JournalLibraryItem { id: i.id.into(), label: i.label.into(), category: i.category.into(), selected: sel.iter().any(|s| s == i.id) })
        .collect()
}

pub fn get_insights(db: &Db) -> Vec<Insight> {
    let today = today_string();
    let from = add_days(&today, -95);
    let entries = db.journal_entries_since(&from);
    let outcomes: Vec<DayOutcome> = db
        .summaries_between(&from, &today)
        .unwrap_or_default()
        .into_iter()
        .map(|s| DayOutcome { day: s.day, recovery: s.recovery, sleep_performance: s.sleep_performance, hrv_ms: s.hrv_ms })
        .collect();
    let computed = behavior_insights(&entries, &outcomes, &today);
    let label = |id: &str| crate::journal::find(id).map(|i| i.label.to_string()).unwrap_or_else(|| id.to_string());
    let mut out: Vec<Insight> = computed
        .iter()
        .map(|b| match &b.status {
            InsightStatus::Ready { yes, no, recovery, sleep_performance, hrv } => Insight {
                behavior: b.behavior.clone(),
                label: label(&b.behavior),
                yes_days: *yes,
                no_days: *no,
                state: "ok".into(),
                recovery_delta: Some(r1(recovery.delta)),
                sleep_delta: sleep_performance.as_ref().map(|e| r1(e.delta)),
                hrv_delta: hrv.as_ref().map(|e| r1(e.delta_pct)),
            },
            InsightStatus::NotEnoughData { yes, no } => Insight {
                behavior: b.behavior.clone(),
                label: label(&b.behavior),
                yes_days: *yes,
                no_days: *no,
                state: "notEnough".into(),
                recovery_delta: None,
                sleep_delta: None,
                hrv_delta: None,
            },
        })
        .collect();
    for id in db.journal_selected() {
        if !out.iter().any(|i| i.behavior == id) {
            out.push(Insight { label: label(&id), behavior: id, yes_days: 0, no_days: 0, state: "notEnough".into(), recovery_delta: None, sleep_delta: None, hrv_delta: None });
        }
    }
    out
}

// ============================================================================ healthspan
fn avg(v: impl Iterator<Item = f64>) -> Option<f64> {
    let v: Vec<f64> = v.collect();
    if v.is_empty() { None } else { Some(v.iter().sum::<f64>() / v.len() as f64) }
}

fn healthspan_inputs(db: &Db, sums: &[DailySummary], from: &str, to: &str, max_hr: f64) -> HealthspanInputs {
    let n_days = sums.len().max(1) as f64;
    let weeks = n_days / 7.0;
    let strength_min: f64 = db
        .activities_between(from, to)
        .unwrap_or_default()
        .iter()
        .filter(|(_, a)| a.kind.to_lowercase().contains("strength"))
        .map(|(_, a)| (a.end_ms - a.start_ms) as f64 / 60_000.0)
        .sum();
    HealthspanInputs {
        sleep_hours: avg(sums.iter().filter_map(|s| s.sleep_asleep_min.map(|m| m as f64 / 60.0))),
        sleep_consistency: avg(sums.iter().filter_map(|s| s.sleep_consistency)),
        steps_per_day: avg(sums.iter().filter_map(|s| s.steps.map(|v| v as f64))),
        zones_1_3_min_per_week: Some(sums.iter().map(|s| (s.zone1_min + s.zone2_min + s.zone3_min) as f64).sum::<f64>() / weeks),
        zones_4_5_min_per_week: Some(sums.iter().map(|s| (s.zone4_min + s.zone5_min) as f64).sum::<f64>() / weeks),
        strength_min_per_week: Some(strength_min / weeks),
        vo2max: avg(sums.iter().filter_map(|s| s.resting_hr.map(|r| estimate_vo2max(max_hr, r)))),
        resting_hr: avg(sums.iter().filter_map(|s| s.resting_hr)),
        lean_mass_pct: None,
    }
}

pub fn get_healthspan(db: &Db) -> R<Avail<HealthspanOut>> {
    let today = today_string();
    let from6 = add_days(&today, -182);
    let sums: Vec<DailySummary> = db.summaries_between(&from6, &today)?.into_iter().filter(|s| s.avg_hr.is_some()).collect();
    if sums.len() < 14 {
        return Ok(Avail::Calibrating {
            nights_remaining: (14 - sums.len()) as u32,
            message: Some("Healthspan needs at least two weeks of data. It is an approximate estimate, not a medical measurement.".into()),
        });
    }
    let profile = db.settings().user_profile();
    let max_hr = unstrapped_core::metrics::hr::max_hr(&profile, &[]);
    let six = healthspan_inputs(db, &sums, &from6, &today, max_hr);
    let from4 = add_days(&today, -27);
    let recent_sums: Vec<DailySummary> = sums.iter().filter(|s| s.day >= from4).cloned().collect();
    let recent = (recent_sums.len() >= 7).then(|| healthspan_inputs(db, &recent_sums, &from4, &today, max_hr));
    let h = healthspan(profile.age, &six, recent.as_ref());
    let fmt = |name: &str, v: Option<f64>| -> (String, &'static str, &'static str) {
        let (unit, target, label) = match name {
            "sleep_duration" => ("h", "7-9 h", "Sleep duration"),
            "sleep_consistency" => ("%", "80% or more", "Sleep consistency"),
            "steps" => ("", "8,000+ a day", "Steps"),
            "zones_1_3" => (" min/wk", "150+ min/wk", "Zones 1-3 time"),
            "zones_4_5" => (" min/wk", "75+ min/wk", "Zones 4-5 time"),
            "strength_time" => (" min/wk", "120+ min/wk", "Strength time"),
            "vo2max" => (" ml/kg/min", "45+", "VO2 max (estimated)"),
            "resting_hr" => (" bpm", "Under 60 bpm", "Resting heart rate"),
            _ => ("%", "75% or more", "Lean mass"),
        };
        let s = v.map(|x| if x >= 100.0 { format!("{}{unit}", x.round() as i64) } else { format!("{}{unit}", r1(x)) }).unwrap_or_else(|| "No data".into());
        (s, target, label)
    };
    let contributors = h
        .contributors
        .iter()
        .map(|c| {
            let (average, target, label) = fmt(&c.name, c.value);
            HealthspanContributorOut { key: c.name.clone(), label: label.into(), average, target: target.into(), years_impact: -r1(c.years) }
        })
        .collect();
    Ok(Avail::ok(HealthspanOut { strap_age: r1(h.physiological_age), chronological_age: r1(h.chronological_age), pace_of_aging: r2(h.pace_of_aging), contributors }))
}

// ============================================================================ raw dashboard
pub fn get_raw_day(db: &Db, day: &str) -> R<RawDay> {
    let (s, e) = day_bounds(day).ok_or("bad day")?;
    let hr = minute_series(db, s, e)?;
    let res = db.day_result(day);
    let acts = db.activities_for_day(day)?;
    let mut spans = vec![];
    if let Some(r) = &res {
        for sl in &r.sleeps {
            spans.push(Span { start_ms: sl.start_ms, end_ms: sl.end_ms, kind: if sl.is_nap { "nap" } else { "sleep" }.into(), label: Some(if sl.is_nap { "Nap" } else { "Sleep" }.into()) });
        }
    }
    for a in acts {
        spans.push(Span { start_ms: a.start_ms, end_ms: a.end_ms, kind: "activity".into(), label: Some(a.kind) });
    }
    // gaps straight from the minute series (runs of >= 5 missing minutes between readings)
    let mut run_start: Option<i64> = None;
    for (i, t) in hr.ts.iter().enumerate() {
        match (hr.v[i], run_start) {
            (None, None) => run_start = Some(*t),
            (Some(_), Some(a)) => {
                if t - a >= 5 * 60_000 {
                    spans.push(Span { start_ms: a, end_ms: *t, kind: "gap".into(), label: Some("Strap was off or out of range".into()) });
                }
                run_start = None;
            }
            _ => {}
        }
    }
    spans.sort_by_key(|s| s.start_ms);
    Ok(RawDay { day: day.into(), hr, spans })
}

pub fn get_raw_night(db: &Db, day: &str) -> R<Avail<RawNight>> {
    let Some(r) = db.day_result(day) else { return Ok(Avail::no_data(crate::screens::NO_DATA_REASON)) };
    let Some(m) = r.sleeps.iter().find(|s| !s.is_nap) else {
        return Ok(Avail::no_data_gaps("No main sleep was detected for this day.", crate::screens::gaps_of(&r)));
    };
    let hr = minute_series(db, m.start_ms, m.end_ms)?;
    let rr = db.rr_range(m.start_ms, m.end_ms)?;
    let step = (rr.len() / 12_000).max(1);
    let mut series = TimeSeries::default();
    for (i, (t, v)) in rr.into_iter().enumerate() {
        if i % step == 0 {
            series.push(t, Some(v as f64));
        }
    }
    Ok(Avail::ok(RawNight { day: day.into(), hr, rr: series }))
}

pub fn get_raw_trends(db: &Db, range: &str) -> R<RawTrends> {
    let days = range_days(range)?;
    let to = today_string();
    let from = add_days(&to, -(days - 1));
    let sums = db.summaries_between(&from, &to)?;
    Ok(RawTrends {
        range: range.into(),
        days: sums.into_iter().map(|s| RawTrendDay { day: s.day, resting_hr: s.resting_hr, hrv: s.hrv_ms }).collect(),
    })
}

// ============================================================================ export
pub fn export_csv(db: &Db, kind: &str, from: &str, to: &str) -> R<String> {
    let (from_ms, _) = day_bounds(from).ok_or("Invalid start day")?;
    let (_, to_ms) = day_bounds(to).ok_or("Invalid end day")?;
    if to_ms <= from_ms {
        return Err("The end day must not be before the start day.".into());
    }
    let home = std::env::var("HOME").map_err(|_| "Cannot find the home folder".to_string())?;
    let dir = std::path::PathBuf::from(home).join("Downloads");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let base = format!("strap-{kind}-{from}_{to}");
    let mut path = dir.join(format!("{base}.csv"));
    let mut n = 1;
    while path.exists() {
        path = dir.join(format!("{base} ({n}).csv"));
        n += 1;
    }
    db.export_csv(kind, from_ms, to_ms, from, to, &path)?;
    Ok(path.to_string_lossy().into_owned())
}
