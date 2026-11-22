//! Protocol-independent input types and the user profile.
//!
//! Units (the data producer must honour these):
//! - timestamps: Unix epoch milliseconds, UTC;
//! - heart rate: beats per minute, ~1 sample per second (at least one per 15 s is needed for
//!   strain/zone accounting; slower streams are under-counted rather than extrapolated);
//! - R-R intervals: milliseconds, `ts_ms` is the time the beat was received/ended. Several beats may
//!   share one timestamp (packetised); beat spacing is taken from the intervals themselves;
//! - motion: `magnitude_g` is the L2 magnitude of the change in the accelerometer/gravity vector
//!   between consecutive samples, in g (about 0.00-0.01 lying still, 0.01-0.05 sitting/desk fidgeting,
//!   0.1+ walking, 0.3+ exercise). Any rate from 0.2 Hz to 25 Hz works; samples are averaged per epoch;
//! - skin temperature: degrees Celsius; SpO2: percent; steps: step count *added* in that sample.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct HrSample {
    pub ts_ms: i64,
    pub bpm: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct RrInterval {
    pub ts_ms: i64,
    pub rr_ms: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct MotionSample {
    pub ts_ms: i64,
    pub magnitude_g: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct TempSample {
    pub ts_ms: i64,
    pub celsius: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct Spo2Sample {
    pub ts_ms: i64,
    pub percent: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct StepSample {
    pub ts_ms: i64,
    pub steps: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Sex {
    Male,
    Female,
    #[default]
    Other,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct UserProfile {
    pub age: f64,
    pub sex: Sex,
    pub weight_kg: f64,
    pub height_cm: Option<f64>,
    pub max_hr_override: Option<f64>,
}

impl Default for UserProfile {
    fn default() -> Self {
        Self { age: 35.0, sex: Sex::Other, weight_kg: 75.0, height_cm: None, max_hr_override: None }
    }
}

/// Journal answer: `behavior` happened (yes) or not on local `day`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct JournalEntry {
    pub day: String,
    pub behavior: String,
    pub answer: bool,
}

/// A time range with no data from the strap.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct DataGap {
    pub start_ms: i64,
    pub end_ms: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Awake,
    Light,
    Deep,
    Rem,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Awake => "awake",
            Stage::Light => "light",
            Stage::Deep => "deep",
            Stage::Rem => "rem",
        }
    }
}

/// A score that is either a value, still calibrating, or unavailable for lack of data. Never a fake 0.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ScoreResult<T> {
    Score(T),
    Calibrating { nights_remaining: u32 },
    Unavailable { reason: String },
}

impl<T> ScoreResult<T> {
    pub fn score(&self) -> Option<&T> {
        match self {
            ScoreResult::Score(t) => Some(t),
            _ => None,
        }
    }
}
