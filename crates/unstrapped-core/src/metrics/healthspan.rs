//! Healthspan: a ROUGH physiological age and pace of aging from nine contributors.
//!
//! APPROXIMATE. The per-contributor year offsets are hand-made piecewise-linear curves loosely shaped like
//! the published risk literature, not a validated model. Treat the output as a motivator, not a measurement.

use serde::{Deserialize, Serialize};

pub const HEALTHSPAN_NOTE: &str = "Approximate and not validated: a rough indicator built from simple curves, not a medical measurement.";

/// Six-month (or recent) averages of the nine contributors. Missing ones contribute 0 years.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct HealthspanInputs {
    pub sleep_hours: Option<f64>,
    /// 0-100
    pub sleep_consistency: Option<f64>,
    pub steps_per_day: Option<f64>,
    pub zones_1_3_min_per_week: Option<f64>,
    pub zones_4_5_min_per_week: Option<f64>,
    pub strength_min_per_week: Option<f64>,
    pub vo2max: Option<f64>,
    pub resting_hr: Option<f64>,
    /// Lean mass as % of body weight.
    pub lean_mass_pct: Option<f64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct HealthspanContributor {
    pub name: String,
    pub value: Option<f64>,
    /// Years added (positive, ages you) or removed (negative).
    pub years: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Healthspan {
    pub approximate: bool,
    pub note: String,
    pub chronological_age: f64,
    pub physiological_age: f64,
    /// Sum of contributor offsets (negative = younger than your age).
    pub offset_years: f64,
    /// Years of physiological ageing per calendar year: `1 + recent_offset / 10`, clamped to 0.5-2.0.
    pub pace_of_aging: f64,
    pub contributors: Vec<HealthspanContributor>,
}

/// Piecewise-linear interpolation, clamped at both ends. `pts` sorted by x.
fn interp(pts: &[(f64, f64)], x: f64) -> f64 {
    if x <= pts[0].0 {
        return pts[0].1;
    }
    for w in pts.windows(2) {
        if x <= w[1].0 {
            let f = (x - w[0].0) / (w[1].0 - w[0].0);
            return w[0].1 + f * (w[1].1 - w[0].1);
        }
    }
    pts[pts.len() - 1].1
}

fn contributors(i: &HealthspanInputs) -> Vec<HealthspanContributor> {
    let c = |name: &str, v: Option<f64>, pts: &[(f64, f64)]| HealthspanContributor { name: name.into(), value: v, years: v.map_or(0.0, |x| interp(pts, x)) };
    vec![
        c("sleep_duration", i.sleep_hours, &[(5.0, 2.5), (6.0, 1.2), (7.0, 0.0), (8.0, -0.8), (9.0, -0.2), (10.0, 1.0)]),
        c("sleep_consistency", i.sleep_consistency, &[(40.0, 1.5), (60.0, 0.5), (80.0, -0.5), (95.0, -1.0)]),
        c("steps", i.steps_per_day, &[(2000.0, 2.5), (5000.0, 0.8), (8000.0, -0.5), (11000.0, -1.5), (14000.0, -1.8)]),
        c("zones_1_3", i.zones_1_3_min_per_week, &[(0.0, 1.0), (150.0, -0.3), (300.0, -0.8), (600.0, -1.0)]),
        c("zones_4_5", i.zones_4_5_min_per_week, &[(0.0, 1.0), (30.0, 0.0), (75.0, -0.8), (150.0, -1.5), (300.0, -1.8)]),
        c("strength_time", i.strength_min_per_week, &[(0.0, 0.8), (60.0, -0.3), (120.0, -0.8), (240.0, -1.0)]),
        c("vo2max", i.vo2max, &[(25.0, 4.0), (35.0, 1.5), (45.0, -1.0), (55.0, -3.0), (65.0, -4.5)]),
        c("resting_hr", i.resting_hr, &[(45.0, -1.5), (55.0, -0.5), (65.0, 0.8), (75.0, 2.0), (85.0, 3.5)]),
        c("lean_mass", i.lean_mass_pct, &[(60.0, 1.0), (75.0, -0.5), (85.0, -1.5)]),
    ]
}

/// Uth-Sorensen estimate: VO2max ~ 15.3 x HRmax / resting HR (ml/kg/min). Rough.
pub fn estimate_vo2max(max_hr: f64, resting_hr: f64) -> f64 {
    15.3 * max_hr / resting_hr
}

/// Physiological age and pace. `six_month` gives the age; `recent` (e.g. last 4 weeks) gives the pace, and
/// defaults to `six_month`.
pub fn healthspan(age: f64, six_month: &HealthspanInputs, recent: Option<&HealthspanInputs>) -> Healthspan {
    let cs = contributors(six_month);
    let offset: f64 = cs.iter().map(|c| c.years).sum();
    let recent_offset: f64 = recent.map_or(offset, |r| contributors(r).iter().map(|c| c.years).sum());
    Healthspan {
        approximate: true,
        note: HEALTHSPAN_NOTE.into(),
        chronological_age: age,
        physiological_age: age + offset,
        offset_years: offset,
        pace_of_aging: (1.0 + recent_offset / 10.0).clamp(0.5, 2.0),
        contributors: cs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolation_hand_worked() {
        // steps 9500: between (8000,-0.5) and (11000,-1.5): f = 0.5 -> -1.0
        let i = HealthspanInputs { steps_per_day: Some(9500.0), ..Default::default() };
        let h = healthspan(40.0, &i, None);
        assert!((h.offset_years + 1.0).abs() < 1e-9);
        assert!((h.physiological_age - 39.0).abs() < 1e-9);
        // pace = 1 + (-1.0)/10 = 0.9
        assert!((h.pace_of_aging - 0.9).abs() < 1e-9);
        assert!(h.approximate);
    }

    #[test]
    fn unhealthy_profile_ages_you_and_pace_is_clamped() {
        let i = HealthspanInputs {
            sleep_hours: Some(5.0),
            sleep_consistency: Some(40.0),
            steps_per_day: Some(2000.0),
            zones_1_3_min_per_week: Some(0.0),
            zones_4_5_min_per_week: Some(0.0),
            strength_min_per_week: Some(0.0),
            vo2max: Some(25.0),
            resting_hr: Some(85.0),
            lean_mass_pct: Some(60.0),
        };
        let h = healthspan(50.0, &i, None);
        // 2.5+1.5+2.5+1+1+0.8+4+3.5+1 = 17.8
        assert!((h.offset_years - 17.8).abs() < 1e-9);
        assert_eq!(h.pace_of_aging, 2.0);
        assert_eq!(h.contributors.len(), 9);
        assert!((estimate_vo2max(190.0, 60.0) - 48.45).abs() < 1e-9);
    }
}
