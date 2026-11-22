//! Health metrics engine: pure, deterministic, DB-free. Methods follow NOOP's documented approach
//! (z-score recovery composite, EWMA baselines, log-scale strain, 55 % sleep-debt carry) re-implemented
//! here; all outputs are approximations from a wrist sensor, not medical measurements.
//!
//! Entry point: [`day::compute_day`].

pub mod activity;
pub mod baselines;
pub mod day;
pub mod healthspan;
pub mod hr;
pub mod insights;
pub mod monitor;
pub mod records;
pub mod scores;
pub mod signals;
pub mod sleep;
pub mod strain;
pub mod stress;
pub mod trends;
pub mod types;
pub mod util;

/// Bump when any formula changes so stored days can be recomputed.
pub const ALGO_VERSION: i64 = 1;

pub use baselines::{BaselineSet, BaselineStat, Calibration};
pub use day::{compute_day, DayInput, DayResult, RawStreams};
pub use records::{Activity, BaselineRow, DailySummary, Sleep, StageSegment, StressPoint};
pub use types::*;
