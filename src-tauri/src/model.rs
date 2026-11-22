//! JSON shapes of docs/API_CONTRACT.md / src/api/types.ts. camelCase everywhere.

use serde::{Deserialize, Serialize};

pub type Ms = i64;
pub type Day = String;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Gap {
    pub start_ms: Ms,
    pub end_ms: Ms,
    pub reason: String,
}

/// The UI's `Result<T>` wrapper (ok / calibrating / noData).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "state", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Avail<T> {
    Ok { data: T },
    Calibrating {
        nights_remaining: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    NoData { reason: String, gaps: Vec<Gap> },
}

impl<T> Avail<T> {
    pub fn ok(data: T) -> Self {
        Avail::Ok { data }
    }
    pub fn no_data(reason: impl Into<String>) -> Self {
        Avail::NoData { reason: reason.into(), gaps: vec![] }
    }
    pub fn no_data_gaps(reason: impl Into<String>, gaps: Vec<Gap>) -> Self {
        Avail::NoData { reason: reason.into(), gaps }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct TimeSeries {
    pub ts: Vec<Ms>,
    pub v: Vec<Option<f64>>,
}

impl TimeSeries {
    pub fn push(&mut self, ts: Ms, v: Option<f64>) {
        self.ts.push(ts);
        self.v.push(v);
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Span {
    pub start_ms: Ms,
    pub end_ms: Ms,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ActivitySummary {
    pub id: String,
    pub kind: String,
    pub start_ms: Ms,
    pub end_ms: Ms,
    pub strain: f64,
    pub avg_hr: f64,
    pub max_hr: f64,
    pub calories: f64,
    pub zone_min: [f64; 5],
    pub manual: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ActivityDetail {
    pub summary: ActivitySummary,
    pub hr: TimeSeries,
    pub zone_bounds: [f64; 4],
    pub max_hr: f64,
}

// ---- home
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FlaggedMetric {
    pub metric: String,
    pub label: String,
    pub status: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HealthMonitorSummary {
    pub in_range: usize,
    pub total: usize,
    pub flagged: Vec<FlaggedMetric>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SleepPerfOnly {
    pub performance: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RecoveryLite {
    pub score: f64,
    pub band: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StrainLite {
    pub value: f64,
    pub band: String,
    pub target_lo: Option<f64>,
    pub target_hi: Option<f64>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StressLite {
    pub current: Option<f64>,
    pub band: Option<String>,
    pub series: TimeSeries,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TonightLite {
    pub need_min: f64,
    pub recommended_bedtime_ms: Ms,
    pub wake_time_ms: Ms,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct KeyStats {
    pub hrv_ms: Option<f64>,
    pub resting_hr: Option<f64>,
    pub resp_rate: Option<f64>,
    pub calories: Option<f64>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HomeData {
    pub day: Day,
    pub sleep: Avail<SleepPerfOnly>,
    pub recovery: Avail<RecoveryLite>,
    pub strain: Avail<StrainLite>,
    pub health_monitor: Avail<HealthMonitorSummary>,
    pub stress: Avail<StressLite>,
    pub tonights_sleep: Avail<TonightLite>,
    pub activities: Vec<ActivitySummary>,
    pub key_stats: Avail<KeyStats>,
    pub gaps: Vec<Gap>,
}

// ---- sleep
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SleepContributor {
    pub key: String,
    pub label: String,
    pub pct: Option<f64>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SleepStageSegment {
    pub start_ms: Ms,
    pub end_ms: Ms,
    pub stage: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SleepNeedOut {
    pub baseline_min: f64,
    pub strain_min: f64,
    pub debt_min: f64,
    pub naps_min: f64,
    pub total_min: f64,
    pub actual_min: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SleepPlannerOption {
    pub goal: String,
    pub goal_pct: f64,
    pub bedtime_ms: Ms,
    pub sleep_min: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PlannerOut {
    pub wake_time_ms: Ms,
    pub typical_efficiency_pct: f64,
    pub options: Vec<SleepPlannerOption>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StageMin {
    pub awake: f64,
    pub light: f64,
    pub deep: f64,
    pub rem: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Restorative {
    pub min: f64,
    pub pct_of_asleep: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SleepStats {
    pub time_in_bed_min: f64,
    pub asleep_min: f64,
    pub efficiency_pct: f64,
    pub latency_min: f64,
    pub disturbances: f64,
    pub resp_rate: Option<f64>,
    pub bedtime_ms: Ms,
    pub wake_ms: Ms,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ConsistencyNight {
    pub day: Day,
    pub bed_ms: Ms,
    pub wake_ms: Ms,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct NapOut {
    pub start_ms: Ms,
    pub end_ms: Ms,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SleepData {
    pub day: Day,
    pub sleep_id: String,
    pub performance: f64,
    pub contributors: Vec<SleepContributor>,
    pub segments: Vec<SleepStageSegment>,
    pub disturbance_times_ms: Vec<Ms>,
    pub stage_min: StageMin,
    pub typical_pct: StageMin,
    pub restorative: Restorative,
    pub stats: SleepStats,
    pub need: SleepNeedOut,
    pub planner: Avail<PlannerOut>,
    pub consistency: Vec<ConsistencyNight>,
    pub naps: Vec<NapOut>,
    pub gaps: Vec<Gap>,
}

// ---- recovery
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryContributor {
    pub key: String,
    pub label: String,
    pub value: f64,
    pub unit: String,
    pub baseline: f64,
    pub delta_pct: f64,
    pub direction: String,
    pub impact_pts: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Last7 {
    pub day: Day,
    pub score: Option<f64>,
    pub band: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryData {
    pub day: Day,
    pub score: f64,
    pub band: String,
    pub reading: String,
    pub contributors: Vec<RecoveryContributor>,
    pub last7: Vec<Last7>,
}

// ---- strain
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StrainStats {
    pub calories: Option<f64>,
    pub avg_hr: Option<f64>,
    pub steps: Option<i64>,
    pub strength_min: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StrainData {
    pub day: Day,
    pub value: f64,
    pub band: String,
    pub target_lo: Option<f64>,
    pub target_hi: Option<f64>,
    pub hr: TimeSeries,
    pub zone_bounds: [f64; 4],
    pub max_hr: f64,
    pub zone_min: [f64; 5],
    pub spans: Vec<Span>,
    pub activities: Vec<ActivitySummary>,
    pub stats: StrainStats,
    pub gaps: Vec<Gap>,
}

// ---- health
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HealthMetricOut {
    pub key: String,
    pub label: String,
    pub value: Option<f64>,
    pub unit: String,
    pub range_lo: Option<f64>,
    pub range_hi: Option<f64>,
    pub status: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EarlyWarning {
    pub message: String,
    pub metrics: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HealthData {
    pub day: Day,
    pub live_hr: Option<f64>,
    pub metrics: Avail<Vec<HealthMetricOut>>,
    pub early_warning: Option<EarlyWarning>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MetricPoint {
    pub day: Day,
    pub value: Option<f64>,
    pub lo: Option<f64>,
    pub hi: Option<f64>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MetricDetail {
    pub metric: String,
    pub label: String,
    pub unit: String,
    pub current: Option<f64>,
    pub range_lo: Option<f64>,
    pub range_hi: Option<f64>,
    pub status: String,
    pub points: Vec<MetricPoint>,
    pub note: String,
}

// ---- stress
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Thresholds {
    pub low_max: f64,
    pub medium_max: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HoursInBand {
    pub low: f64,
    pub medium: f64,
    pub high: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StressData {
    pub day: Day,
    pub current: Option<f64>,
    pub band: Option<String>,
    pub series: TimeSeries,
    pub thresholds: Thresholds,
    pub hours_in_band: HoursInBand,
    pub spans: Vec<Span>,
    pub gaps: Vec<Gap>,
}

// ---- trends
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TrendPoint {
    pub day: Day,
    pub value: Option<f64>,
    pub band: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BaselineBand {
    pub lo: f64,
    pub hi: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DayVal {
    pub day: Day,
    pub value: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TrendSummary {
    pub avg: f64,
    pub change_pct: Option<f64>,
    pub best: DayVal,
    pub worst: DayVal,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TrendData {
    pub metric: String,
    pub label: String,
    pub unit: String,
    pub range: String,
    pub granularity: String,
    pub points: Vec<TrendPoint>,
    pub baseline: Option<BaselineBand>,
    pub summary: Option<TrendSummary>,
}

// ---- journal
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JournalQuestion {
    pub id: String,
    pub label: String,
    pub category: String,
    pub answer: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JournalDay {
    pub day: Day,
    pub questions: Vec<JournalQuestion>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JournalLibraryItem {
    pub id: String,
    pub label: String,
    pub category: String,
    pub selected: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Insight {
    pub behavior: String,
    pub label: String,
    pub yes_days: usize,
    pub no_days: usize,
    pub state: String,
    pub recovery_delta: Option<f64>,
    pub sleep_delta: Option<f64>,
    pub hrv_delta: Option<f64>,
}

// ---- assessments
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AssessmentListItem {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub period_start: Day,
    pub period_end: Day,
    pub created_ms: Ms,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScatterPoint {
    pub day: Day,
    pub strain: f64,
    pub next_recovery: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ZonesByActivity {
    pub kind: String,
    pub zone_min: [f64; 5],
    #[serde(default)]
    pub avg_strain: Option<f64>,
    #[serde(default)]
    pub sessions: usize,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TrainingResponse {
    pub months: Vec<String>,
    pub strain: Vec<Option<f64>>,
    pub recovery: Vec<Option<f64>>,
    pub resting_hr: Vec<Option<f64>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Heat {
    pub recovery: Vec<DayVal>,
    pub strain: Vec<DayVal>,
    pub sleep: Vec<DayVal>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Assessment {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub period_start: Day,
    pub period_end: Day,
    pub avg_recovery: Option<f64>,
    pub avg_strain: Option<f64>,
    pub avg_sleep_performance: Option<f64>,
    pub hours_slept: f64,
    pub hours_needed: f64,
    pub scatter: Vec<ScatterPoint>,
    pub consistency: Vec<ConsistencyNight>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zones_by_activity: Option<Vec<ZonesByActivity>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_journal_impacts: Option<Vec<Insight>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub training_response: Option<TrainingResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heat: Option<Heat>,
}

// ---- healthspan
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HealthspanContributorOut {
    pub key: String,
    pub label: String,
    pub average: String,
    pub target: String,
    pub years_impact: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HealthspanOut {
    pub strap_age: f64,
    pub chronological_age: f64,
    pub pace_of_aging: f64,
    pub contributors: Vec<HealthspanContributorOut>,
}

// ---- raw dashboard
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RawDay {
    pub day: Day,
    pub hr: TimeSeries,
    pub spans: Vec<Span>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RawNight {
    pub day: Day,
    pub hr: TimeSeries,
    pub rr: TimeSeries,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RawTrendDay {
    pub day: Day,
    pub resting_hr: Option<f64>,
    pub hrv: Option<f64>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RawTrends {
    pub range: String,
    pub days: Vec<RawTrendDay>,
}

// ---- strap
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StrapStatus {
    pub transport: String,
    pub state: String,
    pub device_name: Option<String>,
    pub battery_pct: Option<f64>,
    pub charging: bool,
    pub on_wrist: Option<bool>,
    pub last_sync_ms: Option<Ms>,
    pub last_hr: Option<f64>,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SyncProgress {
    pub phase: String,
    pub fraction: f64,
    pub label: String,
}

impl SyncProgress {
    pub fn new(phase: &str, fraction: f64, label: impl Into<String>) -> Self {
        Self { phase: phase.into(), fraction, label: label.into() }
    }
    pub fn idle() -> Self {
        Self::new("idle", 0.0, "")
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RawFrameEvent {
    pub received_at: Ms,
    pub characteristic: String,
    pub hex: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HrEvent {
    pub ts: Ms,
    pub bpm: u16,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DataUpdatedEvent {
    pub days: Vec<Day>,
}

// ---- settings
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Sex {
    Male,
    Female,
    Other,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSettings {
    pub age: f64,
    pub sex: Sex,
    pub weight_kg: f64,
    pub height_cm: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub profile: ProfileSettings,
    pub transport: String,
    pub emulator_address: String,
    pub auto_sync: bool,
    pub launch_at_login: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            profile: ProfileSettings { age: 32.0, sex: Sex::Male, weight_kg: 76.0, height_cm: 180.0 },
            transport: "ble".into(),
            emulator_address: "127.0.0.1:47800".into(),
            auto_sync: true,
            launch_at_login: false,
        }
    }
}

impl Settings {
    pub fn user_profile(&self) -> unstrapped_core::metrics::UserProfile {
        use unstrapped_core::metrics::Sex as S;
        unstrapped_core::metrics::UserProfile {
            age: self.profile.age,
            sex: match self.profile.sex {
                Sex::Male => S::Male,
                Sex::Female => S::Female,
                Sex::Other => S::Other,
            },
            weight_kg: self.profile.weight_kg,
            height_cm: Some(self.profile.height_cm),
            max_hr_override: None,
        }
    }
}
