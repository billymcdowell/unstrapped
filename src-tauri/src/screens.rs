//! One function per screen: reads stored results and returns only what the screen draws.

use crate::compute::{add_days, day_bounds, day_of, today_string};
use crate::db::Db;
use crate::model::*;
use unstrapped_core::metrics::monitor::MonitorStatus;
use unstrapped_core::metrics::strain::{RecoveryBand as RB, StrainBand as SB};
use unstrapped_core::metrics::stress::StressBand;
use unstrapped_core::metrics::{DayResult, ScoreResult, Stage};

type R<T> = Result<T, String>;

pub const NO_DATA_REASON: &str = "No data for this day yet. Sync your strap to fill it in.";
const OFF_REASON: &str = "Strap was off or out of range";

pub fn load(db: &Db, day: &str) -> Option<DayResult> {
    db.day_result(day)
}

pub fn gaps_of(res: &DayResult) -> Vec<Gap> {
    res.gaps.iter().map(|g| Gap { start_ms: g.start_ms, end_ms: g.end_ms, reason: OFF_REASON.into() }).collect()
}

pub fn band_str(b: RB) -> String {
    b.as_str().to_string()
}

pub fn strain_band_str(b: SB) -> &'static str {
    match b {
        SB::Light => "light",
        SB::Moderate => "moderate",
        SB::High => "high",
        SB::AllOut => "allOut",
    }
}

pub fn stress_band_str(b: StressBand) -> &'static str {
    match b {
        StressBand::Low => "low",
        StressBand::Medium => "medium",
        StressBand::High => "high",
    }
}

fn status_str(s: MonitorStatus) -> &'static str {
    match s {
        MonitorStatus::InRange => "ok",
        MonitorStatus::SlightlyOutside => "warn",
        MonitorStatus::WellOutside => "alert",
        MonitorStatus::NoData => "unavailable",
    }
}

fn metric_key(engine: &str) -> &'static str {
    match engine {
        "resp_rate" => "respRate",
        "spo2" => "spo2",
        "resting_hr" => "restingHr",
        "hrv" => "hrv",
        _ => "skinTemp",
    }
}

fn metric_meta(key: &str) -> (&'static str, &'static str) {
    match key {
        "respRate" => ("Respiratory rate", "rpm"),
        "spo2" => ("Blood oxygen", "%"),
        "restingHr" => ("Resting heart rate", "bpm"),
        "hrv" => ("HRV", "ms"),
        _ => ("Skin temperature", "\u{b0}C"),
    }
}

fn r1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

fn convert<T, U>(sr: &ScoreResult<T>, ok: impl FnOnce(&T) -> Avail<U>, gaps: Vec<Gap>) -> Avail<U> {
    match sr {
        ScoreResult::Score(t) => ok(t),
        ScoreResult::Calibrating { nights_remaining } => {
            Avail::Calibrating { nights_remaining: *nights_remaining, message: Some("Your baseline needs a few more nights of data.".into()) }
        }
        ScoreResult::Unavailable { reason } => Avail::NoData { reason: reason.clone(), gaps },
    }
}

/// Per-minute series over `[from, to)` with `null` for minutes without data (so lines break at gaps).
pub fn minute_series(db: &Db, from: i64, to: i64) -> R<TimeSeries> {
    let rows = db.hr_per_minute(from, to)?;
    let mut ts = TimeSeries::default();
    if rows.is_empty() {
        return Ok(ts);
    }
    let first = rows[0].0;
    let last = rows[rows.len() - 1].0;
    let mut it = rows.into_iter().peekable();
    let mut t = first;
    while t <= last {
        match it.peek() {
            Some((m, v)) if *m == t => {
                ts.push(t, Some(r1(*v)));
                it.next();
            }
            _ => ts.push(t, None),
        }
        t += 60_000;
    }
    Ok(ts)
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn day_hr_series(db: &Db, day: &str) -> R<TimeSeries> {
    let (s, e) = day_bounds(day).ok_or("bad day")?;
    minute_series(db, s, e.min(now_ms() + 60_000))
}

fn stress_series(db: &Db, day: &str) -> R<TimeSeries> {
    let (s, e) = day_bounds(day).ok_or("bad day")?;
    let mut ts = TimeSeries::default();
    for (t, v) in db.stress_range(s, e)? {
        ts.push(t, Some(r1(v * 100.0) / 100.0));
    }
    Ok(ts)
}

fn spans_for(res: &Option<DayResult>, acts: &[ActivitySummary], with_gaps: bool) -> Vec<Span> {
    let mut v = vec![];
    if let Some(r) = res {
        for s in &r.sleeps {
            v.push(Span { start_ms: s.start_ms, end_ms: s.end_ms, kind: if s.is_nap { "nap" } else { "sleep" }.into(), label: Some(if s.is_nap { "Nap" } else { "Sleep" }.into()) });
        }
        if with_gaps {
            for g in gaps_of(r) {
                v.push(Span { start_ms: g.start_ms, end_ms: g.end_ms, kind: "gap".into(), label: Some(g.reason) });
            }
        }
    }
    for a in acts {
        v.push(Span { start_ms: a.start_ms, end_ms: a.end_ms, kind: "activity".into(), label: Some(a.kind.clone()) });
    }
    v.sort_by_key(|s| s.start_ms);
    v
}

// ============================================================================ home
pub fn get_home(db: &Db, day: &str) -> R<HomeData> {
    let res = load(db, day);
    let acts = db.activities_for_day(day)?;
    let Some(r) = res.clone() else {
        return Ok(HomeData {
            day: day.into(),
            sleep: Avail::no_data(NO_DATA_REASON),
            recovery: Avail::no_data(NO_DATA_REASON),
            strain: Avail::no_data(NO_DATA_REASON),
            health_monitor: Avail::no_data(NO_DATA_REASON),
            stress: Avail::no_data(NO_DATA_REASON),
            tonights_sleep: Avail::no_data(NO_DATA_REASON),
            activities: acts,
            key_stats: Avail::no_data(NO_DATA_REASON),
            gaps: vec![],
        });
    };
    let gaps = gaps_of(&r);
    let sleep = convert(&r.sleep_performance, |b| Avail::ok(SleepPerfOnly { performance: r1(b.score) }), gaps.clone());
    let recovery = convert(&r.recovery, |b| Avail::ok(RecoveryLite { score: r1(b.score), band: band_str(b.band) }), gaps.clone());
    let strain = match &r.strain {
        Some(s) => Avail::ok(StrainLite {
            value: (s.day_strain * 10.0).round() / 10.0,
            band: strain_band_str(s.band).into(),
            target_lo: s.target.map(|t| t.lo),
            target_hi: s.target.map(|t| t.hi),
        }),
        None => Avail::no_data_gaps("Not enough heart-rate data to calculate strain yet.", gaps.clone()),
    };
    let hm = &r.health_monitor;
    let health_monitor = if hm.total_with_data == 0 {
        match r.calibration {
            unstrapped_core::metrics::Calibration::Calibrating { nights_remaining } => Avail::Calibrating { nights_remaining, message: Some("The Health Monitor needs a few nights to learn your ranges.".into()) },
            _ => Avail::no_data_gaps("No overnight readings for this day.", gaps.clone()),
        }
    } else {
        Avail::ok(HealthMonitorSummary {
            in_range: hm.in_range_count,
            total: hm.total_with_data,
            flagged: hm
                .metrics
                .iter()
                .filter(|m| matches!(m.status, MonitorStatus::SlightlyOutside | MonitorStatus::WellOutside))
                .map(|m| {
                    let k = metric_key(&m.metric);
                    FlaggedMetric { metric: k.into(), label: metric_meta(k).0.into(), status: status_str(m.status).into() }
                })
                .collect(),
        })
    };
    let series = stress_series(db, day)?;
    let stress = if series.ts.is_empty() {
        Avail::no_data_gaps("No stress readings yet today.", gaps.clone())
    } else {
        Avail::ok(StressLite {
            current: r.stress_summary.current.map(|c| (c * 100.0).round() / 100.0),
            band: r.stress_summary.current_band.map(|b| stress_band_str(b).to_string()),
            series,
        })
    };
    let tonights_sleep = match (&r.tonight, &r.tonight_need) {
        (Some(p), Some(n)) => match p.options.first() {
            Some(o) => Avail::ok(TonightLite { need_min: n.total_min.round(), recommended_bedtime_ms: o.bed_ms, wake_time_ms: p.wake_ms }),
            None => Avail::no_data_gaps("No sleep plan available.", gaps.clone()),
        },
        _ => Avail::no_data_gaps("No sleep plan available.", gaps.clone()),
    };
    let s = &r.summary;
    let key_stats = if s.hrv_ms.is_none() && s.resting_hr.is_none() && s.resp_rate.is_none() && s.calories.is_none() {
        Avail::no_data_gaps("No readings for this day.", gaps.clone())
    } else {
        Avail::ok(KeyStats { hrv_ms: s.hrv_ms, resting_hr: s.resting_hr, resp_rate: s.resp_rate, calories: s.calories })
    };
    Ok(HomeData { day: day.into(), sleep, recovery, strain, health_monitor, stress, tonights_sleep, activities: acts, key_stats, gaps })
}

// ============================================================================ sleep
pub fn get_sleep(db: &Db, day: &str) -> R<Avail<SleepData>> {
    let res = load(db, day);
    let Some(r) = res.clone() else { return Ok(Avail::no_data(NO_DATA_REASON)) };
    let gaps = gaps_of(&r);
    let Some(m) = r.sleeps.iter().find(|s| !s.is_nap) else {
        return Ok(Avail::no_data_gaps("No main sleep was detected for this day (the strap may have been off).", gaps));
    };
    let Some(b) = r.sleep_performance.score() else {
        return Ok(convert(&r.sleep_performance, |_| Avail::<SleepData>::no_data(""), gaps));
    };
    let contributors = vec![
        SleepContributor { key: "hoursVsNeeded".into(), label: "Hours vs. Needed".into(), pct: Some(r1(b.hours_vs_needed)) },
        SleepContributor { key: "consistency".into(), label: "Sleep Consistency".into(), pct: b.consistency.map(r1) },
        SleepContributor { key: "efficiency".into(), label: "Sleep Efficiency".into(), pct: Some(r1(b.efficiency)) },
        SleepContributor { key: "sleepStress".into(), label: "Sleep Stress".into(), pct: b.sleep_stress.map(r1) },
    ];
    let segments = m.stages.iter().map(|s| SleepStageSegment { start_ms: s.start_ms, end_ms: s.end_ms, stage: s.stage.as_str().into() }).collect();
    let need = r.sleep_need.as_ref().map(|n| SleepNeedOut {
        baseline_min: n.baseline_min.round(),
        strain_min: n.strain_min.round(),
        debt_min: n.debt_min.round(),
        naps_min: n.naps_min.round(),
        total_min: n.total_min.round(),
        actual_min: m.asleep_min as f64,
    });
    let need = need.unwrap_or(SleepNeedOut { baseline_min: 0.0, strain_min: 0.0, debt_min: 0.0, naps_min: 0.0, total_min: m.asleep_min as f64, actual_min: m.asleep_min as f64 });
    let planner = match &r.tonight {
        Some(p) => Avail::ok(PlannerOut {
            wake_time_ms: p.wake_ms,
            typical_efficiency_pct: (p.typical_efficiency * 100.0).round(),
            options: p
                .options
                .iter()
                .map(|o| SleepPlannerOption {
                    goal: if o.goal == "get_by" { "getBy".into() } else { o.goal.clone() },
                    goal_pct: o.goal_pct,
                    bedtime_ms: o.bed_ms,
                    sleep_min: o.sleep_min.round(),
                })
                .collect(),
        }),
        None => Avail::no_data("No sleep plan available."),
    };
    let from = add_days(day, -13);
    let consistency = db
        .summaries_between(&from, day)?
        .into_iter()
        .filter_map(|s| Some(ConsistencyNight { day: s.day, bed_ms: s.sleep_start_ms?, wake_ms: s.sleep_end_ms? }))
        .collect();
    let naps = r.sleeps.iter().filter(|s| s.is_nap).map(|s| NapOut { start_ms: s.start_ms, end_ms: s.end_ms }).collect();
    Ok(Avail::ok(SleepData {
        day: day.into(),
        sleep_id: m.id.clone(),
        performance: r1(b.score),
        contributors,
        segments,
        disturbance_times_ms: m.disturbance_ts.clone(),
        stage_min: StageMin { awake: m.awake_min as f64, light: m.light_min as f64, deep: m.deep_min as f64, rem: m.rem_min as f64 },
        typical_pct: StageMin { awake: 5.0, light: 50.0, deep: 23.0, rem: 22.0 },
        restorative: Restorative { min: (m.deep_min + m.rem_min) as f64, pct_of_asleep: m.restorative_pct.round() },
        stats: SleepStats {
            time_in_bed_min: m.in_bed_min as f64,
            asleep_min: m.asleep_min as f64,
            efficiency_pct: m.efficiency_pct.round(),
            latency_min: m.latency_min as f64,
            disturbances: m.disturbances as f64,
            resp_rate: m.resp_rate,
            bedtime_ms: m.start_ms,
            wake_ms: m.end_ms,
        },
        need,
        planner,
        consistency,
        naps,
        gaps,
    }))
}

// ============================================================================ recovery
pub fn get_recovery(db: &Db, day: &str) -> R<Avail<RecoveryData>> {
    let res = load(db, day);
    let Some(r) = res.clone() else { return Ok(Avail::no_data(NO_DATA_REASON)) };
    let gaps = gaps_of(&r);
    let prior = db.summaries_before(day, 14)?;
    let sleep_base = {
        let v: Vec<f64> = prior.iter().filter_map(|s| s.sleep_performance).collect();
        if v.is_empty() { None } else { Some(v.iter().sum::<f64>() / v.len() as f64) }
    };
    let from = add_days(day, -6);
    let summaries = db.summaries_between(&from, day)?;
    Ok(convert(
        &r.recovery,
        |b| {
            let mut contributors = vec![];
            for (name, key, label, unit) in [
                ("hrv", "hrv", "HRV", "ms"),
                ("resting_hr", "restingHr", "Resting heart rate", "bpm"),
                ("resp_rate", "respRate", "Respiratory rate", "rpm"),
                ("sleep", "sleepPerformance", "Sleep performance", "%"),
            ] {
                let Some(c) = b.contributors.iter().find(|c| c.name == name) else { continue };
                let baseline = c.baseline.or(sleep_base).unwrap_or(c.value);
                let delta = if baseline.abs() > 1e-9 { (c.value - baseline) / baseline * 100.0 } else { 0.0 };
                let direction = if c.points > 0.05 { "good" } else if c.points < -0.05 { "bad" } else { "neutral" };
                contributors.push(RecoveryContributor {
                    key: key.into(),
                    label: label.into(),
                    value: r1(c.value),
                    unit: unit.into(),
                    baseline: r1(baseline),
                    delta_pct: r1(delta),
                    direction: direction.into(),
                    impact_pts: r1(c.points),
                });
            }
            let reading = match b.band {
                RB::Green => "Green: your body is primed for strain.",
                RB::Yellow => "Yellow: a balanced day is a good fit.",
                RB::Red => "Red: prioritise rest and recovery today.",
            };
            let last7 = (0..7)
                .map(|i| {
                    let d = add_days(&from, i);
                    let s = summaries.iter().find(|s| s.day == d);
                    Last7 { day: d, score: s.and_then(|s| s.recovery), band: s.and_then(|s| s.recovery_band.clone()) }
                })
                .collect();
            Avail::ok(RecoveryData { day: day.into(), score: r1(b.score), band: band_str(b.band), reading: reading.into(), contributors, last7 })
        },
        gaps,
    ))
}

// ============================================================================ strain
pub fn get_strain(db: &Db, day: &str) -> R<Avail<StrainData>> {
    let res = load(db, day);
    let Some(r) = res.clone() else { return Ok(Avail::no_data(NO_DATA_REASON)) };
    let gaps = gaps_of(&r);
    let Some(st) = &r.strain else {
        return Ok(Avail::no_data_gaps("Not enough heart-rate data to calculate strain (needs at least 10 minutes).", gaps));
    };
    let acts = db.activities_for_day(day)?;
    let strength: f64 = acts.iter().filter(|a| a.kind.to_lowercase().contains("strength")).map(|a| (a.end_ms - a.start_ms) as f64 / 60_000.0).sum();
    let s = &r.summary;
    Ok(Avail::ok(StrainData {
        day: day.into(),
        value: (st.day_strain * 10.0).round() / 10.0,
        band: strain_band_str(st.band).into(),
        target_lo: st.target.map(|t| t.lo),
        target_hi: st.target.map(|t| t.hi),
        hr: day_hr_series(db, day)?,
        zone_bounds: [st.zones.lower_bpm[1], st.zones.lower_bpm[2], st.zones.lower_bpm[3], st.zones.lower_bpm[4]].map(|x| x.round()),
        max_hr: st.max_hr.round(),
        zone_min: st.zone_minutes.map(|m| m.round()),
        spans: spans_for(&res, &acts, true),
        activities: acts,
        stats: StrainStats { calories: s.calories, avg_hr: s.avg_hr, steps: s.steps, strength_min: strength.round() },
        gaps,
    }))
}

pub fn zones_for_day(db: &Db, day: &str) -> (unstrapped_core::metrics::hr::HrZones, f64) {
    if let Some(r) = load(db, day)
        && let Some(st) = r.strain {
            return (st.zones, st.max_hr);
        }
    let p = db.settings().user_profile();
    let max = unstrapped_core::metrics::hr::max_hr(&p, &[]);
    (unstrapped_core::metrics::hr::hr_zones(max, 60.0), max)
}

pub fn get_activity(db: &Db, id: &str) -> R<ActivityDetail> {
    let a = db.activity(id)?.ok_or_else(|| format!("Unknown activity {id}"))?;
    let day = db.activity_day(id).unwrap_or_else(|| day_of(a.start_ms));
    let (zones, max) = zones_for_day(db, &day);
    Ok(ActivityDetail {
        hr: minute_series(db, a.start_ms, a.end_ms + 60_000)?,
        zone_bounds: [zones.lower_bpm[1], zones.lower_bpm[2], zones.lower_bpm[3], zones.lower_bpm[4]].map(|x| x.round()),
        max_hr: max.round(),
        summary: a,
    })
}

pub fn add_activity(db: &Db, kind: &str, start_ms: i64, end_ms: i64) -> R<(ActivitySummary, String)> {
    if end_ms <= start_ms {
        return Err("The activity must end after it starts.".into());
    }
    if end_ms - start_ms > 24 * 3_600_000 {
        return Err("An activity can be at most 24 hours long.".into());
    }
    let kind = kind.trim();
    if kind.is_empty() {
        return Err("Give the activity a name.".into());
    }
    let day = day_of(start_ms);
    let profile = db.settings().user_profile();
    let (zones, _) = zones_for_day(db, &day);
    let streams = db.load_streams(start_ms, end_ms)?;
    let m = crate::compute::measure_window(&streams.hr, &profile, &zones, start_ms, end_ms);
    let a = ActivitySummary {
        id: format!("act-manual-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]),
        kind: kind.into(),
        start_ms,
        end_ms,
        strain: m.0,
        avg_hr: m.1,
        max_hr: m.2,
        calories: m.3,
        zone_min: m.4,
        manual: true,
    };
    db.upsert_manual_activity(&day, &a)?;
    Ok((a, day))
}

// ============================================================================ health
pub fn get_health(db: &Db, day: &str, live_hr: Option<f64>) -> R<HealthData> {
    let res = load(db, day);
    let Some(r) = res.clone() else {
        return Ok(HealthData { day: day.into(), live_hr, metrics: Avail::no_data(NO_DATA_REASON), early_warning: None });
    };
    let hm = &r.health_monitor;
    let order = ["respRate", "spo2", "restingHr", "hrv", "skinTemp"];
    let metrics: Vec<HealthMetricOut> = order
        .iter()
        .filter_map(|k| {
            let m = hm.metrics.iter().find(|m| metric_key(&m.metric) == *k)?;
            let (label, unit) = metric_meta(k);
            Some(HealthMetricOut {
                key: (*k).into(),
                label: label.into(),
                value: m.value.map(|v| if *k == "skinTemp" { (v * 100.0).round() / 100.0 } else { r1(v) }),
                unit: unit.into(),
                range_lo: m.range_lo.map(r1),
                range_hi: m.range_hi.map(r1),
                status: status_str(m.status).into(),
            })
        })
        .collect();
    let any_value = metrics.iter().any(|m| m.value.is_some());
    let avail = if !any_value {
        match r.calibration {
            unstrapped_core::metrics::Calibration::Calibrating { nights_remaining } if nights_remaining > 0 && r.summary.hrv_ms.is_none() => {
                Avail::no_data_gaps("No overnight readings for this day.", gaps_of(&r))
            }
            _ => Avail::no_data_gaps("No overnight readings for this day.", gaps_of(&r)),
        }
    } else {
        Avail::ok(metrics)
    };
    let early_warning = hm.early_warning.then(|| EarlyWarning {
        message: "Respiratory rate, resting heart rate and HRV moved together. Consider taking it easy today.".into(),
        metrics: hm
            .early_warning_signals
            .iter()
            .map(|s| match s.as_str() {
                "resp_rate_up" => "respRate",
                "resting_hr_up" => "restingHr",
                "hrv_down" => "hrv",
                _ => "skinTemp",
            })
            .map(String::from)
            .collect(),
    });
    Ok(HealthData { day: day.into(), live_hr, metrics: avail, early_warning })
}

const NOTES: [(&str, &str); 5] = [
    ("respRate", "Illness, altitude, alcohol and heavy training can raise respiratory rate."),
    ("spo2", "Altitude, congestion and poor strap fit can lower blood oxygen readings."),
    ("restingHr", "Illness, alcohol, travel, dehydration and hard training can raise resting heart rate."),
    ("hrv", "Stress, alcohol, illness and poor sleep tend to lower HRV; good recovery raises it."),
    ("skinTemp", "Fever, a hot room or the menstrual cycle can shift skin temperature."),
];

pub fn get_metric_detail(db: &Db, metric: &str, day: &str) -> R<Avail<MetricDetail>> {
    let Some(key) = NOTES.iter().map(|n| n.0).find(|k| *k == metric) else { return Err(format!("Unknown metric '{metric}'")) };
    let Some(r) = load(db, day) else { return Ok(Avail::no_data(NO_DATA_REASON)) };
    let (label, unit) = metric_meta(key);
    let m = r.health_monitor.metrics.iter().find(|m| metric_key(&m.metric) == key);
    let (lo, hi) = (m.and_then(|m| m.range_lo).map(r1), m.and_then(|m| m.range_hi).map(r1));
    let from = add_days(day, -29);
    let sums = db.summaries_between(&from, day)?;
    let points = (0..30)
        .map(|i| {
            let d = add_days(&from, i);
            let s = sums.iter().find(|s| s.day == d);
            let v = s.and_then(|s| match key {
                "respRate" => s.resp_rate,
                "spo2" => s.spo2,
                "restingHr" => s.resting_hr,
                "hrv" => s.hrv_ms,
                _ => s.skin_temp_c,
            });
            MetricPoint { day: d, value: v.map(|v| (v * 100.0).round() / 100.0), lo, hi }
        })
        .collect();
    Ok(Avail::ok(MetricDetail {
        metric: key.into(),
        label: label.into(),
        unit: unit.into(),
        current: m.and_then(|m| m.value).map(r1),
        range_lo: lo,
        range_hi: hi,
        status: m.map(|m| status_str(m.status)).unwrap_or("unavailable").into(),
        points,
        note: NOTES.iter().find(|n| n.0 == key).map(|n| n.1).unwrap_or("").into(),
    }))
}

// ============================================================================ stress
pub fn get_stress(db: &Db, day: &str) -> R<Avail<StressData>> {
    let res = load(db, day);
    let Some(r) = res.clone() else { return Ok(Avail::no_data(NO_DATA_REASON)) };
    let series = stress_series(db, day)?;
    if series.ts.is_empty() {
        return Ok(Avail::no_data_gaps("No stress readings for this day (the strap may have been off).", gaps_of(&r)));
    }
    let sm = &r.stress_summary;
    let acts = db.activities_for_day(day)?;
    Ok(Avail::ok(StressData {
        day: day.into(),
        current: sm.current.map(|c| (c * 100.0).round() / 100.0),
        band: sm.current_band.map(|b| stress_band_str(b).to_string()),
        series,
        thresholds: Thresholds { low_max: 1.0, medium_max: 2.0 },
        hours_in_band: HoursInBand { low: r1(sm.low_min as f64 / 60.0), medium: r1(sm.medium_min as f64 / 60.0), high: r1(sm.high_min as f64 / 60.0) },
        spans: spans_for(&res, &acts, false),
        gaps: gaps_of(&r),
    }))
}

pub fn is_today(day: &str) -> bool {
    day == today_string()
}

#[allow(dead_code)]
fn stage_label(s: Stage) -> &'static str {
    s.as_str()
}
