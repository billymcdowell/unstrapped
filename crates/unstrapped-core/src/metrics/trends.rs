//! Trend helpers: period average, change vs previous period, best/worst day, weekly averages.

use super::util::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct DayValue {
    pub day: String,
    pub value: Option<f64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PeriodSummary {
    pub days: i64,
    pub n: usize,
    pub avg: Option<f64>,
    pub prev_avg: Option<f64>,
    /// `avg - prev_avg`.
    pub change: Option<f64>,
    pub change_pct: Option<f64>,
    pub best: Option<DayValue>,
    pub worst: Option<DayValue>,
}

fn window(series: &[DayValue], end_idx: i64, days: i64) -> Vec<(i64, f64)> {
    series
        .iter()
        .filter_map(|d| {
            let i = parse_day(&d.day)?;
            (i > end_idx - days && i <= end_idx).then_some((i, d.value?))
        })
        .collect()
}

/// Average over the `days` days ending at `end_day` (inclusive), change versus the `days` before that,
/// and best/worst day (`higher_is_better` decides which end is best). Days without a value are skipped.
pub fn period_summary(series: &[DayValue], end_day: &str, days: i64, higher_is_better: bool) -> Option<PeriodSummary> {
    let end = parse_day(end_day)?;
    let cur = window(series, end, days);
    let prev = window(series, end - days, days);
    let avg = mean(&cur.iter().map(|x| x.1).collect::<Vec<_>>());
    let prev_avg = mean(&prev.iter().map(|x| x.1).collect::<Vec<_>>());
    let change = avg.zip(prev_avg).map(|(a, p)| a - p);
    let change_pct = avg.zip(prev_avg).and_then(|(a, p)| (p.abs() > 1e-9).then(|| (a - p) / p * 100.0));
    let pick = |max: bool| {
        cur.iter()
            .copied()
            .reduce(|a, b| if (b.1 > a.1) == max && b.1 != a.1 { b } else { a })
            .map(|(i, v)| DayValue { day: format_day(i), value: Some(v) })
    };
    Some(PeriodSummary {
        days,
        n: cur.len(),
        avg,
        prev_avg,
        change,
        change_pct,
        best: pick(higher_is_better),
        worst: pick(!higher_is_better),
    })
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct WeekAverage {
    /// Monday of the week, `YYYY-MM-DD`.
    pub week_start: String,
    pub avg: f64,
    pub n: usize,
}

/// Average per Monday-based week (for 6M / 1Y trend lines); weeks with no values are omitted.
pub fn weekly_averages(series: &[DayValue]) -> Vec<WeekAverage> {
    let mut m: std::collections::BTreeMap<i64, Vec<f64>> = Default::default();
    for d in series {
        if let (Some(i), Some(v)) = (parse_day(&d.day), d.value) {
            m.entry(week_start(i)).or_default().push(v);
        }
    }
    m.into_iter().map(|(w, v)| WeekAverage { week_start: format_day(w), avg: v.iter().sum::<f64>() / v.len() as f64, n: v.len() }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series(start: &str, vals: &[Option<f64>]) -> Vec<DayValue> {
        let s = parse_day(start).unwrap();
        vals.iter().enumerate().map(|(i, v)| DayValue { day: format_day(s + i as i64), value: *v }).collect()
    }

    #[test]
    fn period_summary_hand_worked() {
        // 14 days from Mon 2026-03-02: first week 50,52,54,56,58,60,62 (avg 56); second week 60,70,80,None,50,90,70
        let v: Vec<Option<f64>> = [50.0, 52.0, 54.0, 56.0, 58.0, 60.0, 62.0, 60.0, 70.0, 80.0].iter().map(|x| Some(*x)).chain([None, Some(50.0), Some(90.0), Some(70.0)]).collect();
        let s = series("2026-03-02", &v);
        let p = period_summary(&s, "2026-03-15", 7, true).unwrap();
        // current avg = (60+70+80+50+90+70)/6 = 70 ; previous = 56 ; change = 14 ; pct = 25
        assert!((p.avg.unwrap() - 70.0).abs() < 1e-9);
        assert!((p.prev_avg.unwrap() - 56.0).abs() < 1e-9);
        assert!((p.change.unwrap() - 14.0).abs() < 1e-9);
        assert!((p.change_pct.unwrap() - 25.0).abs() < 1e-9);
        assert_eq!(p.n, 6);
        assert_eq!(p.best.as_ref().unwrap().value, Some(90.0));
        assert_eq!(p.best.as_ref().unwrap().day, "2026-03-14");
        assert_eq!(p.worst.as_ref().unwrap().value, Some(50.0));
        // lower-is-better (e.g. resting HR) swaps best and worst
        let q = period_summary(&s, "2026-03-15", 7, false).unwrap();
        assert_eq!(q.best.unwrap().value, Some(50.0));
    }

    #[test]
    fn weekly_averaging_groups_on_mondays() {
        let s = series("2026-03-04", &[Some(1.0), Some(3.0), Some(5.0), Some(7.0), Some(9.0)]); // Wed..Sun? 03-04 is Wed
        // Wed,Thu,Fri,Sat,Sun -> week of Mon 03-02 : all five (1+3+5+7+9)/5 = 5
        let w = weekly_averages(&s);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].week_start, "2026-03-02");
        assert!((w[0].avg - 5.0).abs() < 1e-9);
        let s2 = series("2026-03-08", &[Some(2.0), Some(4.0)]); // Sun, then Mon
        let w2 = weekly_averages(&s2);
        assert_eq!(w2.len(), 2);
        assert_eq!(w2[1].week_start, "2026-03-09");
    }

    #[test]
    fn empty_period_has_no_average() {
        let p = period_summary(&[], "2026-03-15", 7, true).unwrap();
        assert!(p.avg.is_none() && p.best.is_none() && p.change.is_none());
    }
}
