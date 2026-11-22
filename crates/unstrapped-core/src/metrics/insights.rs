//! Behaviour insights: difference in mean next-day outcome between "yes" and "no" journal days.

use super::types::JournalEntry;
use super::util::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Minimum yes and no answers (each) before an insight unlocks.
pub const MIN_ANSWERS: usize = 5;
/// Look-back window for journal entries.
pub const WINDOW_DAYS: i64 = 90;

/// Outcomes measured on `day` (e.g. that morning's recovery).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct DayOutcome {
    pub day: String,
    pub recovery: Option<f64>,
    pub sleep_performance: Option<f64>,
    pub hrv_ms: Option<f64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Effect {
    pub mean_yes: f64,
    pub mean_no: f64,
    /// `mean_yes - mean_no` in the metric's units (recovery / sleep performance points, HRV ms).
    pub delta: f64,
    /// `delta / mean_no x 100`.
    pub delta_pct: f64,
    pub n_yes: usize,
    pub n_no: usize,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum InsightStatus {
    Ready { yes: usize, no: usize, recovery: Effect, sleep_performance: Option<Effect>, hrv: Option<Effect> },
    NotEnoughData { yes: usize, no: usize },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct BehaviorInsight {
    pub behavior: String,
    pub status: InsightStatus,
}

fn effect(pairs: &[(bool, f64)]) -> Option<Effect> {
    let yes: Vec<f64> = pairs.iter().filter(|p| p.0).map(|p| p.1).collect();
    let no: Vec<f64> = pairs.iter().filter(|p| !p.0).map(|p| p.1).collect();
    if yes.len() < 3 || no.len() < 3 {
        return None;
    }
    let (my, mn) = (mean(&yes)?, mean(&no)?);
    Some(Effect { mean_yes: my, mean_no: mn, delta: my - mn, delta_pct: if mn.abs() > 1e-9 { (my - mn) / mn * 100.0 } else { 0.0 }, n_yes: yes.len(), n_no: no.len() })
}

/// For each behaviour: pair every answer on day `d` (within the last 90 days up to `today`) with the
/// outcomes of day `d + 1`. With >= 5 yes and >= 5 no (counting days that have a next-day recovery) returns
/// the mean difference for recovery (and sleep performance / HRV when enough data); otherwise
/// `NotEnoughData { yes, no }`. Results are sorted by absolute recovery effect (ready first).
pub fn behavior_insights(entries: &[JournalEntry], outcomes: &[DayOutcome], today: &str) -> Vec<BehaviorInsight> {
    let Some(today_i) = parse_day(today) else { return vec![] };
    let by_day: BTreeMap<i64, &DayOutcome> = outcomes.iter().filter_map(|o| parse_day(&o.day).map(|d| (d, o))).collect();
    let mut groups: BTreeMap<&str, Vec<(bool, Option<&DayOutcome>)>> = BTreeMap::new();
    for e in entries {
        let Some(d) = parse_day(&e.day) else { continue };
        if d > today_i || d <= today_i - WINDOW_DAYS {
            continue;
        }
        groups.entry(e.behavior.as_str()).or_default().push((e.answer, by_day.get(&(d + 1)).copied()));
    }
    let mut out: Vec<BehaviorInsight> = groups
        .into_iter()
        .map(|(name, rows)| {
            let rec: Vec<(bool, f64)> = rows.iter().filter_map(|(a, o)| o.and_then(|o| o.recovery).map(|v| (*a, v))).collect();
            let yes = rec.iter().filter(|p| p.0).count();
            let no = rec.len() - yes;
            let status = match (yes >= MIN_ANSWERS && no >= MIN_ANSWERS).then(|| effect(&rec)).flatten() {
                Some(recovery) => {
                    let sp: Vec<(bool, f64)> = rows.iter().filter_map(|(a, o)| o.and_then(|o| o.sleep_performance).map(|v| (*a, v))).collect();
                    let hv: Vec<(bool, f64)> = rows.iter().filter_map(|(a, o)| o.and_then(|o| o.hrv_ms).map(|v| (*a, v))).collect();
                    InsightStatus::Ready { yes, no, recovery, sleep_performance: effect(&sp), hrv: effect(&hv) }
                }
                None => InsightStatus::NotEnoughData { yes, no },
            };
            BehaviorInsight { behavior: name.to_string(), status }
        })
        .collect();
    out.sort_by(|a, b| {
        let key = |i: &BehaviorInsight| match &i.status {
            InsightStatus::Ready { recovery, .. } => (0, -recovery.delta.abs()),
            InsightStatus::NotEnoughData { .. } => (1, 0.0),
        };
        let (ka, kb) = (key(a), key(b));
        ka.0.cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(day: i64, b: &str, a: bool) -> JournalEntry {
        JournalEntry { day: format_day(day), behavior: b.into(), answer: a }
    }
    fn outcome(day: i64, rec: f64) -> DayOutcome {
        DayOutcome { day: format_day(day), recovery: Some(rec), sleep_performance: Some(rec + 10.0), hrv_ms: Some(rec) }
    }

    #[test]
    fn unlocks_at_5_yes_5_no_and_computes_difference() {
        let base = days_from_civil(2026, 1, 1);
        let mut entries = Vec::new();
        let mut outcomes = Vec::new();
        // alcohol: 6 yes days (next-day recovery 50), 6 no days (next-day 62) => delta -12
        for i in 0..12 {
            let yes = i % 2 == 0;
            entries.push(entry(base + i, "alcohol", yes));
            outcomes.push(outcome(base + i + 1, if yes { 50.0 } else { 62.0 }));
        }
        let r = behavior_insights(&entries, &outcomes, &format_day(base + 20));
        assert_eq!(r.len(), 1);
        match &r[0].status {
            InsightStatus::Ready { yes, no, recovery, sleep_performance, .. } => {
                assert_eq!((*yes, *no), (6, 6));
                // delta = 50 - 62 = -12 ; pct = -12/62*100 = -19.35
                assert!((recovery.delta + 12.0).abs() < 1e-9);
                assert!((recovery.delta_pct + 19.355).abs() < 0.01);
                assert!((sleep_performance.as_ref().unwrap().delta + 12.0).abs() < 1e-9);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn not_enough_data_reports_counts() {
        let base = days_from_civil(2026, 1, 1);
        let mut entries = Vec::new();
        let mut outcomes = Vec::new();
        // 3 yes / 7 no
        for i in 0..10 {
            entries.push(entry(base + i, "sauna", i < 3));
            outcomes.push(outcome(base + i + 1, 60.0));
        }
        let r = behavior_insights(&entries, &outcomes, &format_day(base + 15));
        assert_eq!(r[0].status, InsightStatus::NotEnoughData { yes: 3, no: 7 });
    }

    #[test]
    fn entries_older_than_90_days_or_without_next_day_are_ignored() {
        let base = days_from_civil(2026, 1, 1);
        let mut entries = Vec::new();
        let mut outcomes = Vec::new();
        for i in 0..12 {
            entries.push(entry(base + i, "caffeine", i % 2 == 0));
            outcomes.push(outcome(base + i + 1, 60.0));
        }
        // today 100 days later: all entries fall outside the 90-day window
        assert!(behavior_insights(&entries, &outcomes, &format_day(base + 120)).is_empty());
        // no outcomes at all -> 0/0
        let r = behavior_insights(&entries, &[], &format_day(base + 20));
        assert_eq!(r[0].status, InsightStatus::NotEnoughData { yes: 0, no: 0 });
    }
}
