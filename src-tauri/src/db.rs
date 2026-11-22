//! SQLite storage (rusqlite, WAL). Raw frames are never edited or deleted; every decoded table can be
//! rebuilt from them (`rebuild_decoded`).

use crate::model::*;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use unstrapped_core::metrics::{DailySummary, DayResult};
use unstrapped_core::metrics::{HrSample, MotionSample, RrInterval, Spo2Sample, StepSample, TempSample};
use unstrapped_core::protocol::{DeviceFamily, Frame, HistoricalPacket, HistoryEnd, HistoryRecord, Packet};

pub type DbResult<T> = Result<T, String>;

type MotionRow = (i64, Option<f64>, Option<f64>, Option<f64>, Option<f64>);

fn e<E: std::fmt::Display>(err: E) -> String {
    err.to_string()
}

/// Earliest timestamp we accept for decoded rows (a strap with an unset clock reports 2010 or garbage).
const MIN_PLAUSIBLE_MS: i64 = 1_577_836_800_000; // 2020-01-01

const MIGRATIONS: &[&str] = &[
    // v1
    r#"
CREATE TABLE raw_frames (
  id TEXT PRIMARY KEY,
  received_at INTEGER NOT NULL,
  strap_id TEXT NOT NULL,
  family TEXT NOT NULL DEFAULT 'whoop4',
  channel TEXT NOT NULL,
  kind TEXT NOT NULL,
  bytes BLOB NOT NULL
);
CREATE INDEX raw_frames_received ON raw_frames(received_at);
CREATE UNIQUE INDEX raw_frames_history_uniq ON raw_frames(strap_id, bytes) WHERE kind = 'history';

CREATE TABLE heart_rate (ts_ms INTEGER PRIMARY KEY, bpm INTEGER NOT NULL, frame_id TEXT);
CREATE TABLE rr_intervals (ts_ms INTEGER NOT NULL, idx INTEGER NOT NULL, rr_ms INTEGER NOT NULL, frame_id TEXT, PRIMARY KEY (ts_ms, idx));
CREATE TABLE temperature (ts_ms INTEGER PRIMARY KEY, celsius REAL NOT NULL, raw INTEGER, frame_id TEXT);
CREATE TABLE motion (ts_ms INTEGER PRIMARY KEY, gx REAL, gy REAL, gz REAL, dyn_g REAL, activity INTEGER, frame_id TEXT);
CREATE TABLE spo2_raw (ts_ms INTEGER PRIMARY KEY, red INTEGER NOT NULL, ir INTEGER NOT NULL, frame_id TEXT);
CREATE TABLE steps (ts_ms INTEGER PRIMARY KEY, counter INTEGER NOT NULL, frame_id TEXT);

CREATE TABLE sync_state (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);

CREATE TABLE daily_summary (
  day TEXT PRIMARY KEY,
  recovery REAL, recovery_band TEXT, sleep_performance REAL,
  hours_vs_needed REAL, sleep_consistency REAL, sleep_efficiency REAL, sleep_stress REAL,
  day_strain REAL, strain_target_lo REAL, strain_target_hi REAL,
  hrv_ms REAL, resting_hr REAL, resp_rate REAL, skin_temp_dev_c REAL, spo2 REAL,
  calories REAL, steps INTEGER, avg_hr REAL,
  zone1_min INTEGER, zone2_min INTEGER, zone3_min INTEGER, zone4_min INTEGER, zone5_min INTEGER,
  confidence TEXT, algo_version INTEGER NOT NULL,
  raw_json TEXT NOT NULL
);
CREATE TABLE day_results (day TEXT PRIMARY KEY, json TEXT NOT NULL, computed_ms INTEGER NOT NULL);

CREATE TABLE sleeps (
  id TEXT PRIMARY KEY, day TEXT NOT NULL, start_ms INTEGER, end_ms INTEGER, is_nap INTEGER,
  in_bed_min INTEGER, asleep_min INTEGER, latency_min INTEGER, disturbances INTEGER,
  awake_min INTEGER, light_min INTEGER, deep_min INTEGER, rem_min INTEGER,
  need_baseline_min INTEGER, need_strain_min INTEGER, need_debt_min INTEGER, need_naps_min INTEGER,
  resp_rate REAL, onset_ms INTEGER, efficiency_pct REAL, restorative_pct REAL,
  resting_hr REAL, hrv_ms REAL, gap_min INTEGER, disturbance_ts TEXT
);
CREATE INDEX sleeps_day ON sleeps(day);
CREATE TABLE sleep_stages (sleep_id TEXT NOT NULL, start_ms INTEGER NOT NULL, end_ms INTEGER NOT NULL, stage TEXT NOT NULL);
CREATE INDEX sleep_stages_sleep ON sleep_stages(sleep_id);

CREATE TABLE activities (
  id TEXT PRIMARY KEY, day TEXT NOT NULL, start_ms INTEGER, end_ms INTEGER, kind TEXT,
  auto_detected INTEGER NOT NULL DEFAULT 1,
  strain REAL, avg_hr REAL, max_hr REAL, calories REAL,
  zone1_min INTEGER, zone2_min INTEGER, zone3_min INTEGER, zone4_min INTEGER, zone5_min INTEGER
);
CREATE INDEX activities_day ON activities(day);
CREATE TABLE activity_overrides (id TEXT PRIMARY KEY, kind TEXT NOT NULL);

CREATE TABLE stress (ts_ms INTEGER PRIMARY KEY, score REAL NOT NULL);
CREATE TABLE baselines (day TEXT NOT NULL, metric TEXT NOT NULL, mean REAL, lo REAL, hi REAL, PRIMARY KEY (day, metric));

CREATE TABLE journal (day TEXT NOT NULL, behavior TEXT NOT NULL, answer INTEGER NOT NULL, PRIMARY KEY (day, behavior));
CREATE TABLE journal_behaviors (id TEXT PRIMARY KEY, position INTEGER NOT NULL);

CREATE TABLE assessments (
  id TEXT PRIMARY KEY, kind TEXT NOT NULL, title TEXT NOT NULL,
  period_start TEXT NOT NULL, period_end TEXT NOT NULL, created_ms INTEGER NOT NULL, json TEXT NOT NULL
);
"#,
];

#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

/// What a persisted history chunk added.
#[derive(Debug, Clone, Default)]
pub struct ChunkStats {
    pub frames: usize,
    pub new_frames: usize,
    pub records: usize,
    pub min_ts: Option<i64>,
    pub max_ts: Option<i64>,
}

#[derive(Default, Debug)]
pub struct OwnedStreams {
    pub hr: Vec<HrSample>,
    pub rr: Vec<RrInterval>,
    pub motion: Vec<MotionSample>,
    pub temp: Vec<TempSample>,
    pub spo2: Vec<Spo2Sample>,
    pub steps: Vec<StepSample>,
}

pub fn family_str(f: DeviceFamily) -> &'static str {
    match f {
        DeviceFamily::Whoop4 => "whoop4",
        DeviceFamily::Whoop5 => "whoop5",
    }
}

pub fn family_from_str(s: &str) -> DeviceFamily {
    if s == "whoop5" { DeviceFamily::Whoop5 } else { DeviceFamily::Whoop4 }
}

/// Decode a stored/received type-47 frame into a history record (the single decode path shared by
/// sync and rebuild).
pub fn decode_history_frame(family: DeviceFamily, bytes: &[u8]) -> Option<HistoryRecord> {
    let frame = Frame::decode(bytes, family).ok()?;
    match Packet::decode(&frame, family).ok()? {
        Packet::Historical(HistoricalPacket::Record(r)) => Some(r),
        _ => None,
    }
}

impl Db {
    pub fn open(path: &Path) -> DbResult<Db> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).map_err(e)?;
        }
        let conn = Connection::open(path).map_err(e)?;
        Self::init(conn, true)
    }

    pub fn open_memory() -> DbResult<Db> {
        Self::init(Connection::open_in_memory().map_err(e)?, false)
    }

    fn init(conn: Connection, wal: bool) -> DbResult<Db> {
        if wal {
            conn.pragma_update(None, "journal_mode", "WAL").map_err(e)?;
            conn.pragma_update(None, "synchronous", "FULL").map_err(e)?; // persist-before-ack must be durable
        }
        conn.pragma_update(None, "foreign_keys", "ON").map_err(e)?;
        conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(e)?;
        let db = Db { conn: Arc::new(Mutex::new(conn)) };
        db.migrate()?;
        Ok(db)
    }

    pub fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn schema_version(&self) -> i64 {
        self.conn().pragma_query_value(None, "user_version", |r| r.get(0)).unwrap_or(0)
    }

    fn migrate(&self) -> DbResult<()> {
        let mut conn = self.conn();
        let current: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).map_err(e)?;
        for (i, sql) in MIGRATIONS.iter().enumerate() {
            let v = i as i64 + 1;
            if v <= current {
                continue;
            }
            let tx = conn.transaction().map_err(e)?;
            tx.execute_batch(sql).map_err(e)?;
            tx.pragma_update(None, "user_version", v).map_err(e)?;
            tx.commit().map_err(e)?;
        }
        Ok(())
    }

    // ------------------------------------------------------------ key/value state
    pub fn get_state(&self, key: &str) -> Option<String> {
        self.conn().query_row("SELECT value FROM sync_state WHERE key = ?1", [key], |r| r.get(0)).optional().ok().flatten()
    }

    pub fn set_state(&self, key: &str, value: &str) -> DbResult<()> {
        self.conn()
            .execute("INSERT INTO sync_state(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", params![key, value])
            .map_err(e)?;
        Ok(())
    }

    pub fn last_sync_ms(&self) -> Option<i64> {
        self.get_state("last_sync_ms").and_then(|s| s.parse().ok())
    }

    pub fn get_setting(&self, key: &str) -> Option<String> {
        self.conn().query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional().ok().flatten()
    }

    pub fn set_setting(&self, key: &str, value: &str) -> DbResult<()> {
        self.conn()
            .execute("INSERT INTO settings(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", params![key, value])
            .map_err(e)?;
        Ok(())
    }

    pub fn settings(&self) -> Settings {
        self.get_setting("settings").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save_settings(&self, s: &Settings) -> DbResult<()> {
        self.set_setting("settings", &serde_json::to_string(s).map_err(e)?)
    }

    /// Remembered device id for a transport kind (`ble` / `emulator`).
    pub fn strap_id(&self, kind: &str) -> Option<String> {
        self.get_setting(&format!("strap_id:{kind}"))
    }

    pub fn remember_strap(&self, kind: &str, id: &str) -> DbResult<()> {
        self.set_setting(&format!("strap_id:{kind}"), id)
    }

    // ------------------------------------------------------------ raw + decoded writes
    pub fn insert_live_frames(&self, strap_id: &str, family: DeviceFamily, frames: &[(String, i64, Vec<u8>)]) -> DbResult<()> {
        if frames.is_empty() {
            return Ok(());
        }
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(e)?;
        {
            let mut st = tx
                .prepare_cached("INSERT INTO raw_frames(id, received_at, strap_id, family, channel, kind, bytes) VALUES (?1, ?2, ?3, ?4, ?5, 'live', ?6)")
                .map_err(e)?;
            for (ch, at, bytes) in frames {
                st.execute(params![uuid::Uuid::new_v4().to_string(), at, strap_id, family_str(family), ch, bytes]).map_err(e)?;
            }
        }
        tx.commit().map_err(e)
    }

    /// Persist one acknowledgement-bounded history chunk in ONE transaction: raw frames, decoded rows
    /// and the sync cursor. The caller sends the ack only after this returns `Ok`.
    pub fn persist_history_chunk(
        &self,
        strap_id: &str,
        family: DeviceFamily,
        channel: &str,
        received_at: i64,
        raw_frames: &[Vec<u8>],
        end: &HistoryEnd,
    ) -> DbResult<ChunkStats> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(e)?;
        let mut stats = ChunkStats { frames: raw_frames.len(), ..Default::default() };
        let max_ok = chrono::Utc::now().timestamp_millis() + 2 * 86_400_000;
        {
            for raw in raw_frames {
                let new_id = uuid::Uuid::new_v4().to_string();
                let changed = tx
                    .prepare_cached("INSERT OR IGNORE INTO raw_frames(id, received_at, strap_id, family, channel, kind, bytes) VALUES (?1, ?2, ?3, ?4, ?5, 'history', ?6)")
                    .map_err(e)?
                    .execute(params![new_id, received_at, strap_id, family_str(family), channel, raw])
                    .map_err(e)?;
                let frame_id = if changed > 0 {
                    stats.new_frames += 1;
                    new_id
                } else {
                    tx.query_row("SELECT id FROM raw_frames WHERE strap_id = ?1 AND kind = 'history' AND bytes = ?2", params![strap_id, raw], |r| r.get(0)).map_err(e)?
                };
                if let Some(rec) = decode_history_frame(family, raw) {
                    let ts = rec.sample.unix_ms;
                    if (MIN_PLAUSIBLE_MS..=max_ok).contains(&ts) {
                        insert_decoded(&tx, &rec, &frame_id)?;
                        stats.records += 1;
                        stats.min_ts = Some(stats.min_ts.map_or(ts, |m| m.min(ts)));
                        stats.max_ts = Some(stats.max_ts.map_or(ts, |m| m.max(ts)));
                    }
                }
            }
            let mut set = tx
                .prepare_cached("INSERT INTO sync_state(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
                .map_err(e)?;
            set.execute(params!["cursor_token", hex::encode(&end.ack_token)]).map_err(e)?;
            set.execute(params!["cursor_trim", end.trim_cursor().to_string()]).map_err(e)?;
            set.execute(params!["cursor_unix", end.unix.to_string()]).map_err(e)?;
            set.execute(params!["cursor_at", received_at.to_string()]).map_err(e)?;
        }
        tx.commit().map_err(e)?;
        Ok(stats)
    }

    /// Clear every decoded table and re-decode all stored history frames. Returns (min ts, max ts).
    pub fn rebuild_decoded(&self) -> DbResult<Option<(i64, i64)>> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(e)?;
        for t in ["heart_rate", "rr_intervals", "temperature", "motion", "spo2_raw", "steps"] {
            tx.execute(&format!("DELETE FROM {t}"), []).map_err(e)?;
        }
        let max_ok = chrono::Utc::now().timestamp_millis() + 2 * 86_400_000;
        let mut min = None::<i64>;
        let mut max = None::<i64>;
        {
            let mut st = tx.prepare("SELECT id, family, bytes FROM raw_frames WHERE kind = 'history' ORDER BY received_at, rowid").map_err(e)?;
            let rows: Vec<(String, String, Vec<u8>)> =
                st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(e)?.collect::<Result<_, _>>().map_err(e)?;
            for (id, fam, bytes) in rows {
                if let Some(rec) = decode_history_frame(family_from_str(&fam), &bytes) {
                    let ts = rec.sample.unix_ms;
                    if (MIN_PLAUSIBLE_MS..=max_ok).contains(&ts) {
                        insert_decoded(&tx, &rec, &id)?;
                        min = Some(min.map_or(ts, |m| m.min(ts)));
                        max = Some(max.map_or(ts, |m| m.max(ts)));
                    }
                }
            }
        }
        tx.commit().map_err(e)?;
        Ok(min.zip(max))
    }

    // ------------------------------------------------------------ decoded reads
    pub fn data_bounds(&self) -> Option<(i64, i64)> {
        self.conn()
            .query_row("SELECT MIN(ts_ms), MAX(ts_ms) FROM heart_rate", [], |r| Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, Option<i64>>(1)?)))
            .ok()
            .and_then(|(a, b)| a.zip(b))
    }

    pub fn count(&self, table: &str) -> i64 {
        self.conn().query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0)).unwrap_or(0)
    }

    pub fn load_streams(&self, from_ms: i64, to_ms: i64) -> DbResult<OwnedStreams> {
        let conn = self.conn();
        let mut out = OwnedStreams::default();
        let mut q = conn.prepare("SELECT ts_ms, bpm FROM heart_rate WHERE ts_ms >= ?1 AND ts_ms < ?2 ORDER BY ts_ms").map_err(e)?;
        out.hr = q
            .query_map([from_ms, to_ms], |r| Ok(HrSample { ts_ms: r.get(0)?, bpm: r.get::<_, i64>(1)? as f64 }))
            .map_err(e)?
            .collect::<Result<_, _>>()
            .map_err(e)?;
        let mut q = conn.prepare("SELECT ts_ms, rr_ms FROM rr_intervals WHERE ts_ms >= ?1 AND ts_ms < ?2 ORDER BY ts_ms, idx").map_err(e)?;
        out.rr = q
            .query_map([from_ms, to_ms], |r| Ok(RrInterval { ts_ms: r.get(0)?, rr_ms: r.get::<_, i64>(1)? as f64 }))
            .map_err(e)?
            .collect::<Result<_, _>>()
            .map_err(e)?;
        let mut q = conn.prepare("SELECT ts_ms, celsius FROM temperature WHERE ts_ms >= ?1 AND ts_ms < ?2 ORDER BY ts_ms").map_err(e)?;
        out.temp = q
            .query_map([from_ms, to_ms], |r| Ok(TempSample { ts_ms: r.get(0)?, celsius: r.get(1)? }))
            .map_err(e)?
            .collect::<Result<_, _>>()
            .map_err(e)?;
        // motion: magnitude of the change in the gravity vector between consecutive samples
        let mut q = conn.prepare("SELECT ts_ms, gx, gy, gz, dyn_g FROM motion WHERE ts_ms >= ?1 AND ts_ms < ?2 ORDER BY ts_ms").map_err(e)?;
        let rows: Vec<MotionRow> = q
            .query_map([from_ms, to_ms], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
            .map_err(e)?
            .collect::<Result<_, _>>()
            .map_err(e)?;
        let mut prev: Option<(i64, [f64; 3])> = None;
        for (ts, gx, gy, gz, dyn_g) in rows {
            if let (Some(x), Some(y), Some(z)) = (gx, gy, gz) {
                if let Some((pt, p)) = prev
                    && ts - pt <= 30_000 {
                        let d = ((x - p[0]).powi(2) + (y - p[1]).powi(2) + (z - p[2]).powi(2)).sqrt();
                        out.motion.push(MotionSample { ts_ms: ts, magnitude_g: d });
                    }
                prev = Some((ts, [x, y, z]));
            } else if let Some(d) = dyn_g {
                out.motion.push(MotionSample { ts_ms: ts, magnitude_g: d });
            }
        }
        // steps: the strap reports a cumulative 16-bit counter; the engine wants steps added per sample
        let mut q = conn.prepare("SELECT ts_ms, counter FROM steps WHERE ts_ms >= ?1 AND ts_ms < ?2 ORDER BY ts_ms").map_err(e)?;
        let rows: Vec<(i64, i64)> = q.query_map([from_ms, to_ms], |r| Ok((r.get(0)?, r.get(1)?))).map_err(e)?.collect::<Result<_, _>>().map_err(e)?;
        let mut prev: Option<i64> = None;
        for (ts, c) in rows {
            if let Some(p) = prev {
                let mut d = c - p;
                if d < 0 && p < 65_536 {
                    d += 65_536;
                }
                if d > 0 && d <= 600 {
                    out.steps.push(StepSample { ts_ms: ts, steps: d as u32 });
                }
            }
            prev = Some(c);
        }
        Ok(out)
    }

    /// Per-minute mean heart rate in `[from, to)`, as `(minute_start_ms, bpm)`.
    pub fn hr_per_minute(&self, from_ms: i64, to_ms: i64) -> DbResult<Vec<(i64, f64)>> {
        let conn = self.conn();
        let mut q = conn
            .prepare("SELECT (ts_ms / 60000) * 60000 AS m, AVG(bpm) FROM heart_rate WHERE ts_ms >= ?1 AND ts_ms < ?2 GROUP BY m ORDER BY m")
            .map_err(e)?;
        q.query_map([from_ms, to_ms], |r| Ok((r.get(0)?, r.get(1)?))).map_err(e)?.collect::<Result<_, _>>().map_err(e)
    }

    pub fn rr_range(&self, from_ms: i64, to_ms: i64) -> DbResult<Vec<(i64, i64)>> {
        let conn = self.conn();
        let mut q = conn.prepare("SELECT ts_ms, rr_ms FROM rr_intervals WHERE ts_ms >= ?1 AND ts_ms < ?2 ORDER BY ts_ms, idx").map_err(e)?;
        q.query_map([from_ms, to_ms], |r| Ok((r.get(0)?, r.get(1)?))).map_err(e)?.collect::<Result<_, _>>().map_err(e)
    }

    pub fn latest_hr(&self) -> Option<(i64, i64)> {
        self.conn().query_row("SELECT ts_ms, bpm FROM heart_rate ORDER BY ts_ms DESC LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?))).optional().ok().flatten()
    }

    // ------------------------------------------------------------ computed results
    pub fn summaries_before(&self, day: &str, limit: usize) -> DbResult<Vec<DailySummary>> {
        let conn = self.conn();
        let mut q = conn.prepare("SELECT raw_json FROM daily_summary WHERE day < ?1 ORDER BY day DESC LIMIT ?2").map_err(e)?;
        let rows: Vec<String> = q.query_map(params![day, limit as i64], |r| r.get(0)).map_err(e)?.collect::<Result<_, _>>().map_err(e)?;
        let mut v: Vec<DailySummary> = rows.iter().filter_map(|j| serde_json::from_str(j).ok()).collect();
        v.reverse();
        Ok(v)
    }

    pub fn summaries_between(&self, from: &str, to: &str) -> DbResult<Vec<DailySummary>> {
        let conn = self.conn();
        let mut q = conn.prepare("SELECT raw_json FROM daily_summary WHERE day >= ?1 AND day <= ?2 ORDER BY day").map_err(e)?;
        let rows: Vec<String> = q.query_map(params![from, to], |r| r.get(0)).map_err(e)?.collect::<Result<_, _>>().map_err(e)?;
        Ok(rows.iter().filter_map(|j| serde_json::from_str(j).ok()).collect())
    }

    pub fn last_computed_day(&self) -> Option<String> {
        self.conn().query_row("SELECT MAX(day) FROM daily_summary", [], |r| r.get::<_, Option<String>>(0)).ok().flatten()
    }

    pub fn clear_computed(&self) -> DbResult<()> {
        let conn = self.conn();
        for t in ["daily_summary", "day_results", "sleeps", "sleep_stages", "stress", "baselines"] {
            conn.execute(&format!("DELETE FROM {t}"), []).map_err(e)?;
        }
        conn.execute("DELETE FROM activities WHERE auto_detected = 1", []).map_err(e)?;
        Ok(())
    }

    pub fn day_result(&self, day: &str) -> Option<DayResult> {
        let json: Option<String> = self.conn().query_row("SELECT json FROM day_results WHERE day = ?1", [day], |r| r.get(0)).optional().ok().flatten();
        json.and_then(|j| serde_json::from_str(&j).ok())
    }

    /// Write a computed day into every derived table in one transaction.
    pub fn write_day_result(&self, res: &DayResult, day_start_ms: i64, day_end_ms: i64) -> DbResult<()> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(e)?;
        let s = &res.summary;
        let raw_json = serde_json::to_string(s).map_err(e)?;
        tx.execute(
            "INSERT OR REPLACE INTO daily_summary VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27)",
            params![
                s.day, s.recovery, s.recovery_band, s.sleep_performance, s.hours_vs_needed, s.sleep_consistency, s.sleep_efficiency,
                s.sleep_stress, s.day_strain, s.strain_target_lo, s.strain_target_hi, s.hrv_ms, s.resting_hr, s.resp_rate,
                s.skin_temp_dev_c, s.spo2, s.calories, s.steps, s.avg_hr, s.zone1_min, s.zone2_min, s.zone3_min, s.zone4_min,
                s.zone5_min, s.confidence, s.algo_version, raw_json
            ],
        )
        .map_err(e)?;
        tx.execute(
            "INSERT OR REPLACE INTO day_results(day, json, computed_ms) VALUES (?1, ?2, ?3)",
            params![res.day, serde_json::to_string(res).map_err(e)?, chrono::Utc::now().timestamp_millis()],
        )
        .map_err(e)?;

        // sleeps
        tx.execute("DELETE FROM sleep_stages WHERE sleep_id IN (SELECT id FROM sleeps WHERE day = ?1)", [&res.day]).map_err(e)?;
        tx.execute("DELETE FROM sleeps WHERE day = ?1", [&res.day]).map_err(e)?;
        for sl in &res.sleeps {
            tx.execute(
                "INSERT OR REPLACE INTO sleeps VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25)",
                params![
                    sl.id, res.day, sl.start_ms, sl.end_ms, sl.is_nap, sl.in_bed_min, sl.asleep_min, sl.latency_min, sl.disturbances,
                    sl.awake_min, sl.light_min, sl.deep_min, sl.rem_min, sl.need_baseline_min, sl.need_strain_min, sl.need_debt_min,
                    sl.need_naps_min, sl.resp_rate, sl.onset_ms, sl.efficiency_pct, sl.restorative_pct, sl.resting_hr, sl.hrv_ms,
                    sl.gap_min, serde_json::to_string(&sl.disturbance_ts).map_err(e)?
                ],
            )
            .map_err(e)?;
            for seg in &sl.stages {
                tx.execute("INSERT INTO sleep_stages VALUES (?1,?2,?3,?4)", params![sl.id, seg.start_ms, seg.end_ms, seg.stage.as_str()]).map_err(e)?;
            }
        }

        // auto-detected activities (manual ones are kept), user renames re-applied
        tx.execute("DELETE FROM activities WHERE day = ?1 AND auto_detected = 1", [&res.day]).map_err(e)?;
        for a in &res.activities {
            let kind: String = tx
                .query_row("SELECT kind FROM activity_overrides WHERE id = ?1", [&a.id], |r| r.get(0))
                .optional()
                .map_err(e)?
                .unwrap_or_else(|| a.kind.clone());
            tx.execute(
                "INSERT OR REPLACE INTO activities VALUES (?1,?2,?3,?4,?5,1,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                params![a.id, res.day, a.start_ms, a.end_ms, kind, a.strain, a.avg_hr, a.max_hr, a.calories, a.zone1_min, a.zone2_min, a.zone3_min, a.zone4_min, a.zone5_min],
            )
            .map_err(e)?;
        }

        // stress (every 5 minutes) and baselines for the day
        tx.execute("DELETE FROM stress WHERE ts_ms > ?1 AND ts_ms <= ?2", [day_start_ms, day_end_ms]).map_err(e)?;
        for p in &res.stress {
            tx.execute("INSERT OR REPLACE INTO stress VALUES (?1, ?2)", params![p.ts_ms, p.score]).map_err(e)?;
        }
        tx.execute("DELETE FROM baselines WHERE day = ?1", [&res.day]).map_err(e)?;
        for b in &res.baselines {
            tx.execute("INSERT OR REPLACE INTO baselines VALUES (?1,?2,?3,?4,?5)", params![b.day, b.metric, b.mean, b.lo, b.hi]).map_err(e)?;
        }
        tx.commit().map_err(e)
    }

    // ------------------------------------------------------------ activities
    fn activity_from_row(r: &rusqlite::Row) -> rusqlite::Result<ActivitySummary> {
        Ok(ActivitySummary {
            id: r.get(0)?,
            kind: r.get(1)?,
            start_ms: r.get(2)?,
            end_ms: r.get(3)?,
            strain: r.get(4)?,
            avg_hr: r.get(5)?,
            max_hr: r.get(6)?,
            calories: r.get(7)?,
            zone_min: [r.get::<_, f64>(8)?, r.get::<_, f64>(9)?, r.get::<_, f64>(10)?, r.get::<_, f64>(11)?, r.get::<_, f64>(12)?],
            manual: r.get::<_, i64>(13)? == 0,
        })
    }

    const ACT_COLS: &'static str = "id, kind, start_ms, end_ms, strain, avg_hr, max_hr, calories, zone1_min, zone2_min, zone3_min, zone4_min, zone5_min, auto_detected";

    pub fn activities_for_day(&self, day: &str) -> DbResult<Vec<ActivitySummary>> {
        let conn = self.conn();
        let mut q = conn.prepare(&format!("SELECT {} FROM activities WHERE day = ?1 ORDER BY start_ms", Self::ACT_COLS)).map_err(e)?;
        q.query_map([day], Self::activity_from_row).map_err(e)?.collect::<Result<_, _>>().map_err(e)
    }

    pub fn activities_between(&self, from_day: &str, to_day: &str) -> DbResult<Vec<(String, ActivitySummary)>> {
        let conn = self.conn();
        let mut q = conn
            .prepare(&format!("SELECT {}, day FROM activities WHERE day >= ?1 AND day <= ?2 ORDER BY start_ms", Self::ACT_COLS))
            .map_err(e)?;
        q.query_map([from_day, to_day], |r| Ok((r.get::<_, String>(14)?, Self::activity_from_row(r)?))).map_err(e)?.collect::<Result<_, _>>().map_err(e)
    }

    pub fn activity(&self, id: &str) -> DbResult<Option<ActivitySummary>> {
        let conn = self.conn();
        conn.query_row(&format!("SELECT {} FROM activities WHERE id = ?1", Self::ACT_COLS), [id], Self::activity_from_row).optional().map_err(e)
    }

    pub fn upsert_manual_activity(&self, day: &str, a: &ActivitySummary) -> DbResult<()> {
        self.conn()
            .execute(
                "INSERT OR REPLACE INTO activities VALUES (?1,?2,?3,?4,?5,0,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                params![a.id, day, a.start_ms, a.end_ms, a.kind, a.strain, a.avg_hr, a.max_hr, a.calories, a.zone_min[0], a.zone_min[1], a.zone_min[2], a.zone_min[3], a.zone_min[4]],
            )
            .map_err(e)?;
        Ok(())
    }

    pub fn manual_activities_for_day(&self, day: &str) -> DbResult<Vec<ActivitySummary>> {
        Ok(self.activities_for_day(day)?.into_iter().filter(|a| a.manual).collect())
    }

    pub fn rename_activity(&self, id: &str, kind: &str) -> DbResult<()> {
        let conn = self.conn();
        let n = conn.execute("UPDATE activities SET kind = ?2 WHERE id = ?1", params![id, kind]).map_err(e)?;
        if n == 0 {
            return Err(format!("Unknown activity {id}"));
        }
        conn.execute("INSERT INTO activity_overrides(id, kind) VALUES (?1, ?2) ON CONFLICT(id) DO UPDATE SET kind = excluded.kind", params![id, kind]).map_err(e)?;
        Ok(())
    }

    pub fn activity_day(&self, id: &str) -> Option<String> {
        self.conn().query_row("SELECT day FROM activities WHERE id = ?1", [id], |r| r.get(0)).optional().ok().flatten()
    }

    // ------------------------------------------------------------ stress
    pub fn stress_range(&self, from_ms: i64, to_ms: i64) -> DbResult<Vec<(i64, f64)>> {
        let conn = self.conn();
        let mut q = conn.prepare("SELECT ts_ms, score FROM stress WHERE ts_ms > ?1 AND ts_ms <= ?2 ORDER BY ts_ms").map_err(e)?;
        q.query_map([from_ms, to_ms], |r| Ok((r.get(0)?, r.get(1)?))).map_err(e)?.collect::<Result<_, _>>().map_err(e)
    }

    // ------------------------------------------------------------ journal
    pub fn journal_selected(&self) -> Vec<String> {
        let initialised = self.get_state("journal_initialised").is_some();
        let conn = self.conn();
        let mut q = match conn.prepare("SELECT id FROM journal_behaviors ORDER BY position") {
            Ok(q) => q,
            Err(_) => return vec![],
        };
        let ids: Vec<String> = q.query_map([], |r| r.get(0)).map(|it| it.filter_map(|x| x.ok()).collect()).unwrap_or_default();
        if ids.is_empty() && !initialised {
            return crate::journal::DEFAULT_SELECTED.iter().map(|s| s.to_string()).collect();
        }
        ids
    }

    pub fn set_journal_selected(&self, ids: &[String]) -> DbResult<()> {
        {
            let mut conn = self.conn();
            let tx = conn.transaction().map_err(e)?;
            tx.execute("DELETE FROM journal_behaviors", []).map_err(e)?;
            for (i, id) in ids.iter().enumerate() {
                tx.execute("INSERT OR IGNORE INTO journal_behaviors VALUES (?1, ?2)", params![id, i as i64]).map_err(e)?;
            }
            tx.commit().map_err(e)?;
        }
        self.set_state("journal_initialised", "1")
    }

    pub fn journal_answers(&self, day: &str) -> Vec<(String, bool)> {
        let conn = self.conn();
        let mut q = match conn.prepare("SELECT behavior, answer FROM journal WHERE day = ?1") {
            Ok(q) => q,
            Err(_) => return vec![],
        };
        q.query_map([day], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? != 0))).map(|it| it.filter_map(|x| x.ok()).collect()).unwrap_or_default()
    }

    pub fn set_journal_answer(&self, day: &str, behavior: &str, answer: Option<bool>) -> DbResult<()> {
        let conn = self.conn();
        match answer {
            Some(a) => conn
                .execute(
                    "INSERT INTO journal(day, behavior, answer) VALUES (?1,?2,?3) ON CONFLICT(day, behavior) DO UPDATE SET answer = excluded.answer",
                    params![day, behavior, a as i64],
                )
                .map(|_| ()),
            None => conn.execute("DELETE FROM journal WHERE day = ?1 AND behavior = ?2", params![day, behavior]).map(|_| ()),
        }
        .map_err(e)
    }

    pub fn journal_entries_since(&self, from_day: &str) -> Vec<unstrapped_core::metrics::JournalEntry> {
        let conn = self.conn();
        let mut q = match conn.prepare("SELECT day, behavior, answer FROM journal WHERE day >= ?1") {
            Ok(q) => q,
            Err(_) => return vec![],
        };
        q.query_map([from_day], |r| Ok(unstrapped_core::metrics::JournalEntry { day: r.get(0)?, behavior: r.get(1)?, answer: r.get::<_, i64>(2)? != 0 }))
            .map(|it| it.filter_map(|x| x.ok()).collect())
            .unwrap_or_default()
    }

    // ------------------------------------------------------------ assessments
    pub fn assessment_exists(&self, id: &str) -> bool {
        self.conn().query_row("SELECT 1 FROM assessments WHERE id = ?1", [id], |_| Ok(())).optional().ok().flatten().is_some()
    }

    pub fn insert_assessment(&self, item: &AssessmentListItem, a: &Assessment) -> DbResult<()> {
        self.conn()
            .execute(
                "INSERT OR REPLACE INTO assessments VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![item.id, item.kind, item.title, item.period_start, item.period_end, item.created_ms, serde_json::to_string(a).map_err(e)?],
            )
            .map_err(e)?;
        Ok(())
    }

    pub fn list_assessments(&self) -> DbResult<Vec<AssessmentListItem>> {
        let conn = self.conn();
        let mut q = conn
            .prepare("SELECT id, kind, title, period_start, period_end, created_ms FROM assessments ORDER BY created_ms DESC, period_end DESC")
            .map_err(e)?;
        q.query_map([], |r| {
            Ok(AssessmentListItem { id: r.get(0)?, kind: r.get(1)?, title: r.get(2)?, period_start: r.get(3)?, period_end: r.get(4)?, created_ms: r.get(5)? })
        })
        .map_err(e)?
        .collect::<Result<_, _>>()
        .map_err(e)
    }

    pub fn get_assessment(&self, id: &str) -> DbResult<Assessment> {
        let json: Option<String> = self.conn().query_row("SELECT json FROM assessments WHERE id = ?1", [id], |r| r.get(0)).optional().map_err(e)?;
        let json = json.ok_or_else(|| format!("Unknown assessment {id}"))?;
        serde_json::from_str(&json).map_err(e)
    }

    // ------------------------------------------------------------ CSV export
    pub fn export_csv(&self, kind: &str, from_ms: i64, to_ms: i64, from_day: &str, to_day: &str, out: &Path) -> DbResult<usize> {
        let conn = self.conn();
        let mut w = csv_writer(out)?;
        let mut n = 0usize;
        match kind {
            "heart_rate" => {
                w.row(&["ts_ms", "iso_time", "bpm"])?;
                let mut q = conn.prepare("SELECT ts_ms, bpm FROM heart_rate WHERE ts_ms >= ?1 AND ts_ms < ?2 ORDER BY ts_ms").map_err(e)?;
                let mut rows = q.query([from_ms, to_ms]).map_err(e)?;
                while let Some(r) = rows.next().map_err(e)? {
                    let ts: i64 = r.get(0).map_err(e)?;
                    w.row(&[&ts.to_string(), &iso(ts), &r.get::<_, i64>(1).map_err(e)?.to_string()])?;
                    n += 1;
                }
            }
            "rr_intervals" => {
                w.row(&["ts_ms", "iso_time", "rr_ms"])?;
                let mut q = conn.prepare("SELECT ts_ms, rr_ms FROM rr_intervals WHERE ts_ms >= ?1 AND ts_ms < ?2 ORDER BY ts_ms, idx").map_err(e)?;
                let mut rows = q.query([from_ms, to_ms]).map_err(e)?;
                while let Some(r) = rows.next().map_err(e)? {
                    let ts: i64 = r.get(0).map_err(e)?;
                    w.row(&[&ts.to_string(), &iso(ts), &r.get::<_, i64>(1).map_err(e)?.to_string()])?;
                    n += 1;
                }
            }
            "raw_frames" => {
                w.row(&["id", "received_at_ms", "iso_time", "strap_id", "channel", "kind", "hex"])?;
                let mut q = conn
                    .prepare("SELECT id, received_at, strap_id, channel, kind, bytes FROM raw_frames WHERE received_at >= ?1 AND received_at < ?2 ORDER BY received_at, rowid")
                    .map_err(e)?;
                let mut rows = q.query([from_ms, to_ms]).map_err(e)?;
                while let Some(r) = rows.next().map_err(e)? {
                    let at: i64 = r.get(1).map_err(e)?;
                    let bytes: Vec<u8> = r.get(5).map_err(e)?;
                    w.row(&[
                        &r.get::<_, String>(0).map_err(e)?,
                        &at.to_string(),
                        &iso(at),
                        &r.get::<_, String>(2).map_err(e)?,
                        &r.get::<_, String>(3).map_err(e)?,
                        &r.get::<_, String>(4).map_err(e)?,
                        &hex::encode(bytes),
                    ])?;
                    n += 1;
                }
            }
            "daily_summary" => {
                let cols = "day, recovery, recovery_band, sleep_performance, hours_vs_needed, sleep_consistency, sleep_efficiency, sleep_stress, day_strain, strain_target_lo, strain_target_hi, hrv_ms, resting_hr, resp_rate, skin_temp_dev_c, spo2, calories, steps, avg_hr, zone1_min, zone2_min, zone3_min, zone4_min, zone5_min, confidence, algo_version";
                n = dump_query(&conn, &mut w, &format!("SELECT {cols} FROM daily_summary WHERE day >= ?1 AND day <= ?2 ORDER BY day"), cols, &[from_day, to_day])?;
            }
            "sleeps" => {
                let cols = "id, day, start_ms, end_ms, is_nap, in_bed_min, asleep_min, latency_min, disturbances, awake_min, light_min, deep_min, rem_min, need_baseline_min, need_strain_min, need_debt_min, need_naps_min, resp_rate, efficiency_pct, restorative_pct, resting_hr, hrv_ms";
                n = dump_query(&conn, &mut w, &format!("SELECT {cols} FROM sleeps WHERE day >= ?1 AND day <= ?2 ORDER BY start_ms"), cols, &[from_day, to_day])?;
            }
            "activities" => {
                let cols = "id, day, kind, start_ms, end_ms, auto_detected, strain, avg_hr, max_hr, calories, zone1_min, zone2_min, zone3_min, zone4_min, zone5_min";
                n = dump_query(&conn, &mut w, &format!("SELECT {cols} FROM activities WHERE day >= ?1 AND day <= ?2 ORDER BY start_ms"), cols, &[from_day, to_day])?;
            }
            "stress" => {
                w.row(&["ts_ms", "iso_time", "score"])?;
                let mut q = conn.prepare("SELECT ts_ms, score FROM stress WHERE ts_ms >= ?1 AND ts_ms < ?2 ORDER BY ts_ms").map_err(e)?;
                let mut rows = q.query([from_ms, to_ms]).map_err(e)?;
                while let Some(r) = rows.next().map_err(e)? {
                    let ts: i64 = r.get(0).map_err(e)?;
                    w.row(&[&ts.to_string(), &iso(ts), &r.get::<_, f64>(1).map_err(e)?.to_string()])?;
                    n += 1;
                }
            }
            other => return Err(format!("Unknown export kind '{other}'")),
        }
        w.finish()?;
        Ok(n)
    }
}

fn iso(ts_ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ts_ms).map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)).unwrap_or_default()
}

struct Csv {
    w: std::io::BufWriter<std::fs::File>,
}

fn csv_writer(path: &Path) -> DbResult<Csv> {
    Ok(Csv { w: std::io::BufWriter::new(std::fs::File::create(path).map_err(e)?) })
}

impl Csv {
    fn row<S: AsRef<str>>(&mut self, cells: &[S]) -> DbResult<()> {
        use std::io::Write;
        let line: Vec<String> = cells
            .iter()
            .map(|c| {
                let c = c.as_ref();
                if c.contains([',', '"', '\n']) { format!("\"{}\"", c.replace('"', "\"\"")) } else { c.to_string() }
            })
            .collect();
        writeln!(self.w, "{}", line.join(",")).map_err(e)
    }
    fn finish(mut self) -> DbResult<()> {
        use std::io::Write;
        self.w.flush().map_err(e)
    }
}

fn dump_query(conn: &Connection, w: &mut Csv, sql: &str, cols: &str, args: &[&str]) -> DbResult<usize> {
    let headers: Vec<&str> = cols.split(',').map(|s| s.trim()).collect();
    w.row(&headers)?;
    let mut q = conn.prepare(sql).map_err(e)?;
    let mut rows = q.query(rusqlite::params_from_iter(args.iter())).map_err(e)?;
    let mut n = 0;
    while let Some(r) = rows.next().map_err(e)? {
        let mut cells = Vec::with_capacity(headers.len());
        for i in 0..headers.len() {
            use rusqlite::types::ValueRef;
            cells.push(match r.get_ref(i).map_err(e)? {
                ValueRef::Null => String::new(),
                ValueRef::Integer(v) => v.to_string(),
                ValueRef::Real(v) => v.to_string(),
                ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned(),
                ValueRef::Blob(b) => hex::encode(b),
            });
        }
        w.row(&cells)?;
        n += 1;
    }
    Ok(n)
}

fn insert_decoded(tx: &rusqlite::Transaction, rec: &HistoryRecord, frame_id: &str) -> DbResult<()> {
    let s = &rec.sample;
    let ts = s.unix_ms;
    if let Some(b) = s.bpm {
        tx.prepare_cached("INSERT OR IGNORE INTO heart_rate(ts_ms, bpm, frame_id) VALUES (?1, ?2, ?3)").map_err(e)?.execute(params![ts, b as i64, frame_id]).map_err(e)?;
    }
    for (i, rr) in s.rr_ms.iter().enumerate() {
        tx.prepare_cached("INSERT OR IGNORE INTO rr_intervals(ts_ms, idx, rr_ms, frame_id) VALUES (?1, ?2, ?3, ?4)")
            .map_err(e)?
            .execute(params![ts, i as i64, *rr as i64, frame_id])
            .map_err(e)?;
    }
    if let Some(t) = s.skin_temp_c {
        tx.prepare_cached("INSERT OR IGNORE INTO temperature(ts_ms, celsius, raw, frame_id) VALUES (?1, ?2, ?3, ?4)")
            .map_err(e)?
            .execute(params![ts, t as f64, s.skin_temp_raw.map(|r| r as i64), frame_id])
            .map_err(e)?;
    }
    if let Some(m) = &s.motion {
        let g = m.gravity;
        tx.prepare_cached("INSERT OR IGNORE INTO motion(ts_ms, gx, gy, gz, dyn_g, activity, frame_id) VALUES (?1,?2,?3,?4,?5,?6,?7)")
            .map_err(e)?
            .execute(params![ts, g.map(|v| v[0] as f64), g.map(|v| v[1] as f64), g.map(|v| v[2] as f64), m.dynamic_accel_g.map(|v| v as f64), m.activity.map(|v| v as i64), frame_id])
            .map_err(e)?;
    }
    if let Some(sp) = &s.spo2 {
        tx.prepare_cached("INSERT OR IGNORE INTO spo2_raw(ts_ms, red, ir, frame_id) VALUES (?1,?2,?3,?4)")
            .map_err(e)?
            .execute(params![ts, sp.red as i64, sp.ir as i64, frame_id])
            .map_err(e)?;
    }
    if let Some(c) = s.steps {
        tx.prepare_cached("INSERT OR IGNORE INTO steps(ts_ms, counter, frame_id) VALUES (?1,?2,?3)").map_err(e)?.execute(params![ts, c as i64, frame_id]).map_err(e)?;
    }
    Ok(())
}
