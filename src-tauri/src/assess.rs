//! Weekly (Mondays), monthly (1st) and yearly (1 Jan) assessments, generated once and kept.

use crate::compute::{add_days, day_bounds, days_range};
use crate::db::Db;
use crate::model::*;
use std::collections::BTreeMap;
use unstrapped_core::metrics::util::*;
use unstrapped_core::metrics::DailySummary;

fn r1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

fn mean_of(v: impl Iterator<Item = f64>) -> Option<f64> {
    let v: Vec<f64> = v.collect();
    if v.is_empty() { None } else { Some(r1(v.iter().sum::<f64>() / v.len() as f64)) }
}

/// Build the assessment document for `[start, end]` (inclusive days).
pub fn build(db: &Db, kind: &str, id: &str, title: &str, start: &str, end: &str) -> Result<Assessment, String> {
    let sums: Vec<DailySummary> = db.summaries_between(start, end)?;
    let consistency = sums
        .iter()
        .rev()
        .take(14)
        .filter_map(|s| Some(ConsistencyNight { day: s.day.clone(), bed_ms: s.sleep_start_ms?, wake_ms: s.sleep_end_ms? }))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let mut scatter = vec![];
    for w in sums.windows(2) {
        if parse_day(&w[1].day) == parse_day(&w[0].day).map(|d| d + 1)
            && let (Some(st), Some(rec)) = (w[0].day_strain, w[1].recovery) {
                scatter.push(ScatterPoint { day: w[0].day.clone(), strain: st, next_recovery: rec });
            }
    }
    let mut a = Assessment {
        id: id.into(),
        kind: kind.into(),
        title: title.into(),
        period_start: start.into(),
        period_end: end.into(),
        avg_recovery: mean_of(sums.iter().filter_map(|s| s.recovery)),
        avg_strain: mean_of(sums.iter().filter_map(|s| s.day_strain)),
        avg_sleep_performance: mean_of(sums.iter().filter_map(|s| s.sleep_performance)),
        hours_slept: r1(sums.iter().filter_map(|s| s.sleep_asleep_min).sum::<i64>() as f64 / 60.0),
        hours_needed: r1(sums.iter().filter(|s| s.sleep_asleep_min.is_some()).filter_map(|s| s.sleep_need_min).sum::<i64>() as f64 / 60.0),
        scatter,
        consistency,
        zones_by_activity: None,
        top_journal_impacts: None,
        training_response: None,
        heat: None,
    };
    if kind != "weekly" {
        let mut by_kind: BTreeMap<String, (ZonesByActivity, f64)> = BTreeMap::new();
        for (_, act) in db.activities_between(start, end)? {
            let e = by_kind.entry(act.kind.clone()).or_insert((ZonesByActivity { kind: act.kind.clone(), zone_min: [0.0; 5], avg_strain: None, sessions: 0 }, 0.0));
            for i in 0..5 {
                e.0.zone_min[i] += act.zone_min[i];
            }
            e.0.sessions += 1;
            e.1 += act.strain;
        }
        a.zones_by_activity = Some(
            by_kind
                .into_values()
                .map(|(mut z, strain_sum)| {
                    z.avg_strain = Some(r1(strain_sum / z.sessions as f64));
                    z
                })
                .collect(),
        );
        a.top_journal_impacts = Some(crate::reports::get_insights(db).into_iter().filter(|i| i.state == "ok").take(4).collect());
        // training response: last 6 calendar months ending at `end`
        let end_idx = parse_day(end).unwrap_or(0);
        let (ey, em, _) = civil_from_days(end_idx);
        let mut months: Vec<(i64, u32)> = vec![];
        let (mut y, mut m) = (ey, em);
        for _ in 0..6 {
            months.push((y, m));
            if m == 1 { y -= 1; m = 12 } else { m -= 1 }
        }
        months.reverse();
        let first = format!("{:04}-{:02}-01", months[0].0, months[0].1);
        let all = db.summaries_between(&first, end)?;
        let names = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
        let per = |f: &dyn Fn(&DailySummary) -> Option<f64>| -> Vec<Option<f64>> {
            months.iter().map(|(y, m)| mean_of(all.iter().filter(|s| s.day.starts_with(&format!("{y:04}-{m:02}-"))).filter_map(f))).collect()
        };
        a.training_response = Some(TrainingResponse {
            months: months.iter().map(|(_, m)| names[*m as usize - 1].to_string()).collect(),
            strain: per(&|s| s.day_strain),
            recovery: per(&|s| s.recovery),
            resting_hr: per(&|s| s.resting_hr),
        });
    }
    if kind == "yearly" {
        let pick = |f: &dyn Fn(&DailySummary) -> Option<f64>| sums.iter().filter_map(|s| Some(DayVal { day: s.day.clone(), value: f(s)? })).collect::<Vec<_>>();
        a.heat = Some(Heat { recovery: pick(&|s| s.recovery), strain: pick(&|s| s.day_strain), sleep: pick(&|s| s.sleep_performance) });
    }
    Ok(a)
}

#[allow(clippy::too_many_arguments)]
fn create(db: &Db, kind: &str, id: String, title: &str, start: &str, end: &str, min_days: usize, out: &mut Vec<AssessmentListItem>) {
    if db.assessment_exists(&id) {
        return;
    }
    let have = days_range(start, end).iter().filter(|d| db.day_result(d).is_some()).count();
    if have < min_days {
        return;
    }
    let Ok(a) = build(db, kind, &id, title, start, end) else { return };
    let item = AssessmentListItem {
        id,
        kind: kind.into(),
        title: title.into(),
        period_start: start.into(),
        period_end: end.into(),
        created_ms: chrono::Utc::now().timestamp_millis(),
    };
    if db.insert_assessment(&item, &a).is_ok() {
        out.push(item);
    }
}

/// Create any assessment that is due as of local `today` and does not exist yet. Returns the new ones.
pub fn generate_due(db: &Db, today: &str) -> Vec<AssessmentListItem> {
    let mut out = vec![];
    let Some(t) = parse_day(today) else { return out };
    // weekly: the Monday-Sunday week before the latest Monday
    let mon = week_start(t);
    let (ws, we) = (format_day(mon - 7), format_day(mon - 1));
    create(db, "weekly", format!("weekly-{ws}"), "Weekly assessment", &ws, &we, 3, &mut out);
    // monthly: the calendar month before the current one
    let (y, m, _) = civil_from_days(t);
    let (py, pm) = if m == 1 { (y - 1, 12) } else { (y, m - 1) };
    let ms = format!("{py:04}-{pm:02}-01");
    let me = add_days(&format!("{y:04}-{m:02}-01"), -1);
    create(db, "monthly", format!("monthly-{py:04}-{pm:02}"), "Monthly assessment", &ms, &me, 7, &mut out);
    // yearly: the previous calendar year
    let ys = format!("{:04}-01-01", y - 1);
    let ye = format!("{:04}-12-31", y - 1);
    create(db, "yearly", format!("yearly-{}", y - 1), "Year in review", &ys, &ye, 60, &mut out);
    let _ = day_bounds;
    out
}
