//! Output records. Field names match the plan 9.4 SQLite columns (plus a few `extra` fields the
//! engine needs back as history; persist the whole struct as JSON or add the columns).

use super::types::*;
use serde::{Deserialize, Serialize};

/// One row of `daily_summary` (+ history extras marked "extra").
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct DailySummary {
    pub day: String,
    pub recovery: Option<f64>,
    pub recovery_band: Option<String>,
    pub sleep_performance: Option<f64>,
    pub hours_vs_needed: Option<f64>,
    pub sleep_consistency: Option<f64>,
    pub sleep_efficiency: Option<f64>,
    pub sleep_stress: Option<f64>,
    pub day_strain: Option<f64>,
    pub strain_target_lo: Option<f64>,
    pub strain_target_hi: Option<f64>,
    pub hrv_ms: Option<f64>,
    pub resting_hr: Option<f64>,
    pub resp_rate: Option<f64>,
    pub skin_temp_dev_c: Option<f64>,
    pub spo2: Option<f64>,
    pub calories: Option<f64>,
    pub steps: Option<i64>,
    pub avg_hr: Option<f64>,
    pub zone1_min: i64,
    pub zone2_min: i64,
    pub zone3_min: i64,
    pub zone4_min: i64,
    pub zone5_min: i64,
    pub confidence: String,
    pub algo_version: i64,
    // ---- extras (feed back as history) ----
    /// extra: absolute median sleeping skin temperature (deg C); baselines are built on this.
    #[serde(default)]
    pub skin_temp_c: Option<f64>,
    /// extra: median heart rate over still, awake 5-min windows (daytime stress reference).
    #[serde(default)]
    pub stress_ref_hr: Option<f64>,
    /// extra: median RMSSD over still, awake 5-min windows.
    #[serde(default)]
    pub stress_ref_hrv: Option<f64>,
    /// extra: main sleep in-bed start / end (Unix ms).
    #[serde(default)]
    pub sleep_start_ms: Option<i64>,
    #[serde(default)]
    pub sleep_end_ms: Option<i64>,
    #[serde(default)]
    pub sleep_asleep_min: Option<i64>,
    #[serde(default)]
    pub sleep_in_bed_min: Option<i64>,
    /// extra: minutes of naps that ended on this day.
    #[serde(default)]
    pub nap_min: i64,
    /// extra: total sleep need (min) the main sleep was scored against.
    #[serde(default)]
    pub sleep_need_min: Option<i64>,
    /// extra: debt carried into the next night after this one (55% of the unmet need).
    #[serde(default)]
    pub sleep_debt_out_min: Option<i64>,
    /// extra: set while the recovery baseline is calibrating.
    #[serde(default)]
    pub calibration_nights_remaining: Option<u32>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct StageSegment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub stage: Stage,
}

/// One row of `sleeps` plus its `sleep_stages`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Sleep {
    pub id: String,
    /// In-bed start (includes the lying-still period before sleep onset).
    pub start_ms: i64,
    pub end_ms: i64,
    pub is_nap: bool,
    pub in_bed_min: i64,
    pub asleep_min: i64,
    pub latency_min: i64,
    pub disturbances: i64,
    pub awake_min: i64,
    pub light_min: i64,
    pub deep_min: i64,
    pub rem_min: i64,
    pub need_baseline_min: Option<i64>,
    pub need_strain_min: Option<i64>,
    pub need_debt_min: Option<i64>,
    pub need_naps_min: Option<i64>,
    pub resp_rate: Option<f64>,
    // extras
    pub onset_ms: i64,
    pub efficiency_pct: f64,
    /// (deep + REM) / asleep, percent.
    pub restorative_pct: f64,
    pub resting_hr: Option<f64>,
    pub hrv_ms: Option<f64>,
    /// Minutes inside the window with no HR data (strap off / dropout), excluded from stages.
    pub gap_min: i64,
    pub stages: Vec<StageSegment>,
    /// Start times of wake events after onset (hypnogram ticks).
    pub disturbance_ts: Vec<i64>,
}

/// One row of `activities`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Activity {
    pub id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub kind: String,
    pub auto_detected: bool,
    pub strain: f64,
    pub avg_hr: f64,
    pub max_hr: f64,
    pub calories: f64,
    pub zone1_min: i64,
    pub zone2_min: i64,
    pub zone3_min: i64,
    pub zone4_min: i64,
    pub zone5_min: i64,
}

/// One row of `stress`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct StressPoint {
    pub ts_ms: i64,
    pub score: f64,
}

/// One row of `baselines`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct BaselineRow {
    pub day: String,
    pub metric: String,
    pub mean: f64,
    pub lo: f64,
    pub hi: f64,
}
