//! Recovery, Sleep Performance, sleep need / debt, Sleep Planner.

use super::baselines::*;
use super::records::DailySummary;
use super::strain::RecoveryBand;
use super::types::*;
use super::util::*;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------- Recovery

pub const W_HRV: f64 = 0.55;
pub const W_RHR: f64 = 0.20;
pub const W_SLEEP: f64 = 0.15;
pub const W_RESP: f64 = 0.05;
pub const W_TEMP: f64 = 0.05;
/// score = 100 / (1 + exp(-K (z - Z0))); z = 0 anchors at ~58 %.
pub const LOGISTIC_K: f64 = 1.6;
pub const LOGISTIC_Z0: f64 = -0.20;
/// Sleep performance is centred at 85 % with a 12-point spread.
const SLEEP_CENTER: f64 = 0.85;
const SLEEP_SPREAD: f64 = 0.12;

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct RecoveryInputs {
    pub hrv_ms: Option<f64>,
    pub resting_hr: Option<f64>,
    pub resp_rate: Option<f64>,
    /// 0-100
    pub sleep_performance: Option<f64>,
    /// Absolute median sleeping skin temperature (deg C).
    pub skin_temp_c: Option<f64>,
}

/// One input's share of the recovery score ("what moved your score").
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Contribution {
    /// `hrv`, `resting_hr`, `resp_rate`, `sleep`, `skin_temp`.
    pub name: String,
    pub value: f64,
    pub baseline: Option<f64>,
    /// Standardised deviation, positive = good for recovery (inverted for RHR/resp, penalty for temp).
    pub z: f64,
    /// Weight after renormalising over the available inputs (sums to 1).
    pub weight: f64,
    /// Signed points added to / removed from the "average day" anchor (~58); sums to `score - anchor`.
    pub points: f64,
    pub good: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct RecoveryBreakdown {
    pub score: f64,
    pub band: RecoveryBand,
    pub z_mean: f64,
    /// Score at z = 0 (the anchor the contributions are measured from).
    pub anchor: f64,
    pub contributors: Vec<Contribution>,
}

pub fn recovery_band(score: f64) -> RecoveryBand {
    if score < 34.0 {
        RecoveryBand::Red
    } else if score < 67.0 {
        RecoveryBand::Yellow
    } else {
        RecoveryBand::Green
    }
}

fn recovery_curve(z: f64) -> f64 {
    100.0 * logistic(LOGISTIC_K * (z - LOGISTIC_Z0))
}

/// Recovery 0-100 from HRV (55), resting HR (20), sleep (15), respiratory rate (5), skin temp (5), each
/// standardised against the personal baseline, weight-averaged (missing inputs drop out and weights are
/// renormalised) and squashed with a logistic. With fewer than 4 valid baseline nights returns
/// `Calibrating`; without HRV returns `Unavailable`.
pub fn recovery(inp: &RecoveryInputs, b: &BaselineSet, valid_nights: u32) -> ScoreResult<RecoveryBreakdown> {
    if valid_nights < MIN_NIGHTS {
        return ScoreResult::Calibrating { nights_remaining: MIN_NIGHTS - valid_nights };
    }
    let (Some(hrv), Some(hb)) = (inp.hrv_ms, b.hrv) else {
        return ScoreResult::Unavailable { reason: "no HRV measured for the night".into() };
    };
    let zc = |z: f64| clamp(z, -3.0, 3.0);
    // (name, value, baseline mean, z, weight)
    let mut items: Vec<(&str, f64, Option<f64>, f64, f64)> = vec![("hrv", hrv, Some(hb.mean), zc(hb.z(hrv)), W_HRV)];
    if let (Some(v), Some(bs)) = (inp.resting_hr, b.resting_hr) {
        items.push(("resting_hr", v, Some(bs.mean), zc((bs.mean - v) / bs.sigma), W_RHR));
    }
    if let Some(sp) = inp.sleep_performance {
        items.push(("sleep", sp, None, zc((sp / 100.0 - SLEEP_CENTER) / SLEEP_SPREAD), W_SLEEP));
    }
    if let (Some(v), Some(bs)) = (inp.resp_rate, b.resp_rate) {
        items.push(("resp_rate", v, Some(bs.mean), zc((bs.mean - v) / bs.sigma), W_RESP));
    }
    if let (Some(v), Some(bs)) = (inp.skin_temp_c, b.skin_temp) {
        // symmetric penalty beyond a half-sigma dead band
        let z = -((v - bs.mean).abs() / bs.sigma - 0.5).max(0.0);
        items.push(("skin_temp", v, Some(bs.mean), zc(z), W_TEMP));
    }
    let wsum: f64 = items.iter().map(|i| i.4).sum();
    let zbar: f64 = items.iter().map(|i| i.4 / wsum * i.3).sum();
    let score = clamp(recovery_curve(zbar), 0.0, 100.0);
    let anchor = recovery_curve(0.0);
    // distribute (score - anchor) in proportion to each weighted z; fall back to the local slope near 0
    let factor = if zbar.abs() > 1e-6 {
        (score - anchor) / zbar
    } else {
        let s = logistic(LOGISTIC_K * LOGISTIC_Z0.abs());
        100.0 * LOGISTIC_K * s * (1.0 - s)
    };
    let contributors = items
        .iter()
        .map(|(name, v, base, z, w)| {
            let wn = w / wsum;
            Contribution {
                name: name.to_string(),
                value: *v,
                baseline: *base,
                z: *z,
                weight: wn,
                points: factor * wn * z,
                good: *z >= 0.0,
            }
        })
        .collect();
    ScoreResult::Score(RecoveryBreakdown { score, band: recovery_band(score), z_mean: zbar, anchor, contributors })
}

// ---------------------------------------------------------------- Sleep need / debt

pub const DEFAULT_NEED_MIN: f64 = 480.0;
/// Fraction of each unmet-need shortfall carried to the next night.
pub const DEBT_CARRY: f64 = 0.55;
/// Debt below this many minutes counts as balanced.
pub const DEBT_DEADBAND_MIN: f64 = 10.0;
const BASELINE_LEARN_NIGHTS: usize = 28;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct SleepNeed {
    pub baseline_min: f64,
    pub strain_min: f64,
    pub debt_min: f64,
    pub naps_min: f64,
    /// baseline + strain + debt - naps (never below 4 h).
    pub total_min: f64,
}

/// Extra sleep for a day's strain: 0 below strain 10, rising linearly to +45 min at 21.
pub fn strain_sleep_extra_min(strain: f64) -> f64 {
    if strain <= 10.0 { 0.0 } else { 45.0 * (clamp(strain, 10.0, 21.0) - 10.0) / 11.0 }
}

pub fn sleep_need(baseline_min: f64, strain: f64, debt_min: f64, naps_min: f64) -> SleepNeed {
    let strain_min = strain_sleep_extra_min(strain);
    SleepNeed {
        baseline_min,
        strain_min,
        debt_min,
        naps_min,
        total_min: (baseline_min + strain_min + debt_min - naps_min).max(240.0),
    }
}

/// Baseline sleep need learned over the first ~28 nights: starts at 8 h, then blends toward the mean of
/// the longer half of the recent nights (your "unconstrained" sleep); clamped to 6.5-9 h.
pub fn baseline_sleep_need(asleep_history_min: &[f64]) -> f64 {
    let v: Vec<f64> = asleep_history_min.iter().rev().take(BASELINE_LEARN_NIGHTS).copied().filter(|x| *x >= 120.0).collect();
    if v.is_empty() {
        return DEFAULT_NEED_MIN;
    }
    let mut s = v.clone();
    s.sort_by(|a, b| b.total_cmp(a));
    let top = &s[..s.len().div_ceil(2)];
    let learned = top.iter().sum::<f64>() / top.len() as f64;
    let n = v.len() as f64;
    let w = n / BASELINE_LEARN_NIGHTS as f64;
    clamp(DEFAULT_NEED_MIN * (1.0 - w) + learned * w, 390.0, 540.0)
}

/// Debt carried out of one night: `0.55 x max(0, need - slept)`, balanced (0) when below 10 min.
pub fn debt_after_night(need_min: f64, credited_sleep_min: f64) -> f64 {
    let d = DEBT_CARRY * (need_min - credited_sleep_min).max(0.0);
    if d < DEBT_DEADBAND_MIN { 0.0 } else { d }
}

/// Recurrence over chronological `(asleep_min, nap_min)` nights (last 14 used):
/// `need = base + debt; debt = 0.55 x max(0, need - asleep - naps)`.
pub fn sleep_debt_from_nights(baseline_min: f64, nights: &[(f64, f64)]) -> f64 {
    let start = nights.len().saturating_sub(14);
    nights[start..].iter().fold(0.0, |debt, (asleep, naps)| debt_after_night(baseline_min + debt, asleep + naps))
}

/// Debt entering the night that ends on `day`: stored carry from the previous day, else the recurrence.
pub fn debt_entering(history: &[DailySummary], day: &str, baseline_min: f64) -> f64 {
    let mut h: Vec<&DailySummary> = history.iter().filter(|d| d.day.as_str() < day).collect();
    h.sort_by(|a, b| a.day.cmp(&b.day));
    if let Some(d) = h.last().and_then(|l| l.sleep_debt_out_min) {
        return d as f64;
    }
    let nights: Vec<(f64, f64)> = h.iter().filter_map(|d| d.sleep_asleep_min.map(|a| (a as f64, d.nap_min as f64))).collect();
    sleep_debt_from_nights(baseline_min, &nights)
}

// ---------------------------------------------------------------- Sleep Performance

pub const SP_W_HOURS: f64 = 0.55;
pub const SP_W_CONSISTENCY: f64 = 0.15;
pub const SP_W_EFFICIENCY: f64 = 0.15;
pub const SP_W_STRESS: f64 = 0.15;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SleepPerformanceBreakdown {
    pub score: f64,
    /// Each 0-100.
    pub hours_vs_needed: f64,
    pub consistency: Option<f64>,
    pub efficiency: f64,
    pub sleep_stress: Option<f64>,
    pub need: SleepNeed,
    pub asleep_min: f64,
    /// True when a contributor was missing and the others were re-weighted.
    pub reweighted: bool,
}

/// Hours vs Needed 55 / Consistency 15 / Efficiency 15 / Sleep Stress 15. Missing contributors drop out
/// and the remaining weights are renormalised (flagged by `reweighted`).
pub fn sleep_performance(
    asleep_min: f64,
    need: SleepNeed,
    consistency: Option<f64>,
    efficiency_pct: f64,
    sleep_stress: Option<f64>,
) -> SleepPerformanceBreakdown {
    let hours = clamp(asleep_min / need.total_min * 100.0, 0.0, 100.0);
    let eff = clamp(efficiency_pct, 0.0, 100.0);
    let parts = [
        (Some(hours), SP_W_HOURS),
        (consistency, SP_W_CONSISTENCY),
        (Some(eff), SP_W_EFFICIENCY),
        (sleep_stress, SP_W_STRESS),
    ];
    let wsum: f64 = parts.iter().filter(|p| p.0.is_some()).map(|p| p.1).sum();
    let score = parts.iter().filter_map(|(v, w)| v.map(|v| v * w)).sum::<f64>() / wsum;
    SleepPerformanceBreakdown {
        score,
        hours_vs_needed: hours,
        consistency,
        efficiency: eff,
        sleep_stress,
        need,
        asleep_min,
        reweighted: parts.iter().any(|p| p.0.is_none()),
    }
}

/// Sleep Consistency 0-100: mean circular absolute deviation (minutes) of bed times and of wake times
/// around their own means; `100 x (1 - dev/180)`. Needs >= 3 nights.
pub fn sleep_consistency(bed_clock_min: &[f64], wake_clock_min: &[f64]) -> Option<f64> {
    if bed_clock_min.len() < 3 || wake_clock_min.len() < 3 {
        return None;
    }
    let dev = (circular_mad_min(bed_clock_min)? + circular_mad_min(wake_clock_min)?) / 2.0;
    Some(clamp(100.0 * (1.0 - dev / 180.0), 0.0, 100.0))
}

/// Sleep Stress contributor from the share (0..1) of sleep spent in the high stress band:
/// `100 x (1 - min(1, 2 x share))`.
pub fn sleep_stress_score(high_share: f64) -> f64 {
    100.0 * (1.0 - (2.0 * high_share).min(1.0))
}

// ---------------------------------------------------------------- Sleep Planner

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PlannerOption {
    /// `peak`, `perform`, `get_by`.
    pub goal: String,
    pub goal_pct: f64,
    pub sleep_min: f64,
    pub in_bed_min: f64,
    /// Local clock time of the suggested bedtime, minutes since midnight.
    pub bed_clock_min: f64,
    pub bed_ms: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SleepPlanner {
    pub need_min: f64,
    pub wake_ms: i64,
    pub typical_efficiency: f64,
    pub options: Vec<PlannerOption>,
}

/// Bedtimes for Peak (100 % of need), Perform (85 %) and Get by (75 %):
/// `bed = wake - need x goal / efficiency`.
pub fn sleep_planner(need_min: f64, wake_ms: i64, efficiency: f64, tz_offset_min: i32) -> SleepPlanner {
    let eff = clamp(efficiency, 0.7, 1.0);
    let options = [("peak", 1.0), ("perform", 0.85), ("get_by", 0.75)]
        .iter()
        .map(|(g, pct)| {
            let sleep = need_min * pct;
            let in_bed = sleep / eff;
            let bed_ms = wake_ms - (in_bed * MS_PER_MIN as f64).round() as i64;
            PlannerOption {
                goal: g.to_string(),
                goal_pct: pct * 100.0,
                sleep_min: sleep,
                in_bed_min: in_bed,
                bed_clock_min: local_clock_min(bed_ms, tz_offset_min),
                bed_ms,
            }
        })
        .collect();
    SleepPlanner { need_min, wake_ms, typical_efficiency: eff, options }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bs(mean: f64, sigma: f64) -> BaselineStat {
        BaselineStat { mean, sigma, lo: mean - sigma, hi: mean + sigma, nights: 14 }
    }

    fn base() -> BaselineSet {
        BaselineSet {
            hrv: Some(bs(60.0, 5.0)),
            resting_hr: Some(bs(52.0, 2.0)),
            resp_rate: Some(bs(15.0, 0.5)),
            skin_temp: Some(bs(33.0, 0.3)),
            ..Default::default()
        }
    }

    #[test]
    fn recovery_hand_worked() {
        // z: HRV (70-60)/5 = 2; RHR (52-50)/2 = 1; resp (15-14)/0.5 = 2; sleep (0.95-0.85)/0.12 = 0.8333; temp dev 0 -> 0
        // zbar = .55*2 + .2*1 + .05*2 + .15*0.8333 + .05*0 = 1.1+0.2+0.1+0.125 = 1.525
        // score = 100/(1+exp(-1.6*(1.525+0.2))) = 100/(1+exp(-2.76)) = 100/(1+0.06329) = 94.05
        let inp = RecoveryInputs { hrv_ms: Some(70.0), resting_hr: Some(50.0), resp_rate: Some(14.0), sleep_performance: Some(95.0), skin_temp_c: Some(33.0) };
        let r = recovery(&inp, &base(), 14);
        let r = r.score().unwrap();
        assert!((r.z_mean - 1.525).abs() < 1e-6, "{}", r.z_mean);
        assert!((r.score - 94.05).abs() < 0.05, "{}", r.score);
        assert_eq!(r.band, RecoveryBand::Green);
        // contributions sum to score - anchor (anchor = 100/(1+exp(-0.32)) = 57.93)
        assert!((r.anchor - 57.93).abs() < 0.01);
        let sum: f64 = r.contributors.iter().map(|c| c.points).sum();
        assert!((sum - (r.score - r.anchor)).abs() < 1e-9);
        // HRV dominates: 1.1/1.525 of the lift
        let hrv = r.contributors.iter().find(|c| c.name == "hrv").unwrap();
        assert!((hrv.points / sum - 1.1 / 1.525).abs() < 1e-6);
        assert!(r.contributors.iter().all(|c| c.good));
    }

    #[test]
    fn recovery_baseline_day_is_about_58_and_bad_day_is_red() {
        let inp = RecoveryInputs { hrv_ms: Some(60.0), resting_hr: Some(52.0), resp_rate: Some(15.0), sleep_performance: Some(85.0), skin_temp_c: Some(33.0) };
        let r = recovery(&inp, &base(), 20);
        let r = r.score().unwrap();
        assert!((r.score - 57.93).abs() < 0.01);
        assert_eq!(r.band, RecoveryBand::Yellow);
        // HRV 45 (z=-3), RHR 58 (z=-3), resp 16.5 (z=-3), sleep 50% (z=-2.9167), temp +1.0 (|dev|/0.3 = 3.33 -> -2.83)
        // all ~ -3 => zbar ~ -2.99 -> score = 100/(1+exp(1.6*2.79)) = 1.1
        let bad = RecoveryInputs { hrv_ms: Some(45.0), resting_hr: Some(58.0), resp_rate: Some(16.5), sleep_performance: Some(50.0), skin_temp_c: Some(34.0) };
        let b = recovery(&bad, &base(), 20);
        let b = b.score().unwrap();
        assert!(b.score < 2.0 && b.band == RecoveryBand::Red);
        assert!(b.contributors.iter().all(|c| !c.good && c.points < 0.0));
    }

    #[test]
    fn recovery_bands() {
        assert_eq!(recovery_band(33.9), RecoveryBand::Red);
        assert_eq!(recovery_band(34.0), RecoveryBand::Yellow);
        assert_eq!(recovery_band(66.9), RecoveryBand::Yellow);
        assert_eq!(recovery_band(67.0), RecoveryBand::Green);
    }

    #[test]
    fn recovery_calibrating_and_missing_hrv() {
        let inp = RecoveryInputs { hrv_ms: Some(60.0), ..Default::default() };
        assert_eq!(recovery(&inp, &base(), 1), ScoreResult::Calibrating { nights_remaining: 3 });
        assert_eq!(recovery(&inp, &BaselineSet::default(), 0), ScoreResult::Calibrating { nights_remaining: 4 });
        assert!(matches!(recovery(&RecoveryInputs::default(), &base(), 10), ScoreResult::Unavailable { .. }));
    }

    #[test]
    fn recovery_reweights_when_inputs_missing() {
        // Only HRV (z=+1) and RHR (z=+1): weights .55/.75 and .2/.75, zbar = 1 -> score 100/(1+exp(-1.92)) = 87.2
        let inp = RecoveryInputs { hrv_ms: Some(65.0), resting_hr: Some(50.0), ..Default::default() };
        let r = recovery(&inp, &base(), 14);
        let r = r.score().unwrap();
        assert!((r.z_mean - 1.0).abs() < 1e-9);
        assert!((r.score - 87.2).abs() < 0.1, "{}", r.score);
        assert!((r.contributors.iter().map(|c| c.weight).sum::<f64>() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn strain_extra_for_sleep() {
        assert_eq!(strain_sleep_extra_min(5.0), 0.0);
        assert_eq!(strain_sleep_extra_min(10.0), 0.0);
        // strain 15.5: 45 x 5.5/11 = 22.5
        assert!((strain_sleep_extra_min(15.5) - 22.5).abs() < 1e-9);
        assert!((strain_sleep_extra_min(21.0) - 45.0).abs() < 1e-9);
    }

    #[test]
    fn need_components() {
        // 480 base + 22.5 strain + 66 debt - 30 nap = 538.5
        let n = sleep_need(480.0, 15.5, 66.0, 30.0);
        assert!((n.total_min - 538.5).abs() < 1e-9);
        assert_eq!(sleep_need(200.0, 0.0, 0.0, 0.0).total_min, 240.0);
    }

    #[test]
    fn debt_recurrence_hand_worked() {
        // base 480. night1 asleep 360: need 480, short 120 -> debt .55*120 = 66
        // night2 asleep 420: need 546, short 126 -> 69.3
        // night3 asleep 540: need 549.3, short 9.3 -> 5.115 -> < 10 -> 0
        assert!((debt_after_night(480.0, 360.0) - 66.0).abs() < 1e-9);
        assert!((sleep_debt_from_nights(480.0, &[(360.0, 0.0), (420.0, 0.0)]) - 69.3).abs() < 1e-9);
        assert_eq!(sleep_debt_from_nights(480.0, &[(360.0, 0.0), (420.0, 0.0), (540.0, 0.0)]), 0.0);
        // naps count as credited sleep: 360 + 120 nap clears it
        assert_eq!(sleep_debt_from_nights(480.0, &[(360.0, 120.0)]), 0.0);
    }

    #[test]
    fn baseline_need_learning() {
        assert_eq!(baseline_sleep_need(&[]), 480.0);
        // 14 nights alternating 420/480: top half = 480 -> w = 14/28 = 0.5 -> 480*.5 + 480*.5 = 480
        let v: Vec<f64> = (0..14).map(|i| if i % 2 == 0 { 420.0 } else { 480.0 }).collect();
        assert!((baseline_sleep_need(&v) - 480.0).abs() < 1e-9);
        // 28 nights of 420: fully learned 420 (above the 390 floor)
        assert!((baseline_sleep_need(&[420.0; 28]) - 420.0).abs() < 1e-9);
        assert_eq!(baseline_sleep_need(&[300.0; 28]), 390.0);
    }

    #[test]
    fn sleep_performance_hand_worked() {
        let need = sleep_need(480.0, 0.0, 0.0, 0.0);
        // asleep 420/480 = 87.5; consistency 80; efficiency 90; stress 100
        // = .55*87.5 + .15*80 + .15*90 + .15*100 = 48.125 + 12 + 13.5 + 15 = 88.625
        let sp = sleep_performance(420.0, need, Some(80.0), 90.0, Some(100.0));
        assert!((sp.score - 88.625).abs() < 1e-9);
        assert!(!sp.reweighted);
        // consistency missing: (48.125 + 13.5 + 15)/0.85 = 90.147
        let sp2 = sleep_performance(420.0, need, None, 90.0, Some(100.0));
        assert!((sp2.score - 90.147).abs() < 0.001);
        assert!(sp2.reweighted);
        // oversleeping caps hours at 100
        assert_eq!(sleep_performance(600.0, need, None, 100.0, None).hours_vs_needed, 100.0);
    }

    #[test]
    fn consistency_hand_worked() {
        // identical times -> 100
        assert!((sleep_consistency(&[1380.0; 4], &[420.0; 4]).unwrap() - 100.0).abs() < 1e-9);
        // beds 22:30, 23:30, 23:00 -> mean 23:00, MAD (30+30+0)/3 = 20; wake const -> dev (20+0)/2 = 10 -> 100*(1-10/180) = 94.44
        let c = sleep_consistency(&[1350.0, 1410.0, 1380.0], &[420.0; 3]).unwrap();
        assert!((c - 94.444).abs() < 0.01, "{c}");
        assert!(sleep_consistency(&[1380.0, 1380.0], &[420.0, 420.0]).is_none());
    }

    #[test]
    fn sleep_stress_contributor() {
        assert_eq!(sleep_stress_score(0.0), 100.0);
        assert!((sleep_stress_score(0.1) - 80.0).abs() < 1e-9);
        assert_eq!(sleep_stress_score(0.6), 0.0);
    }

    #[test]
    fn planner_hand_worked() {
        // need 480, wake 07:00 (420), efficiency 0.9
        // peak: 480/0.9 = 533.3 in bed -> 420 - 533.3 = -113.3 -> 22:06.7 (1326.7)
        // perform: 408/0.9 = 453.3 -> -33.3 -> 23:26.7 (1406.7); get by: 360/0.9 = 400 -> 20 -> 00:20
        let wake = 1_000_000 * MS_PER_DAY + 7 * 3_600_000;
        let p = sleep_planner(480.0, wake, 0.9, 0);
        assert_eq!(p.options.len(), 3);
        assert!((p.options[0].bed_clock_min - 1326.67).abs() < 0.1);
        assert!((p.options[1].bed_clock_min - 1406.67).abs() < 0.1);
        assert!((p.options[2].bed_clock_min - 20.0).abs() < 0.1);
        assert!(p.options[0].bed_ms < p.options[1].bed_ms && p.options[1].bed_ms < p.options[2].bed_ms);
    }
}
