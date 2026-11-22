use crate::db::{Db, decode_history_frame};
use crate::emitter::{CollectEmitter, Emitter};
use crate::model::*;
use crate::strap::{Inbound, Link, StrapManager};
use crate::sync::{SyncOptions, run_history_sync};
use crate::transport::{ConnPhase, ConnectInfo, StrapTransport, TResult, TransportEvent};
use async_trait::async_trait;
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use unstrapped_core::protocol::{
    CodecOptions, DeviceFamily, HistoryEnd, HistoryMetadata, HistoryRecord, HistorySample, HostCommand, Packet, command_number,
};

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn record_frame(family: DeviceFamily, index: u32, ts_ms: i64, bpm: u8) -> Vec<u8> {
    let sample = HistorySample { unix_ms: ts_ms, bpm: Some(bpm), rr_ms: vec![800, 810], skin_temp_c: Some(33.0), skin_temp_raw: Some(826), ..Default::default() };
    let rec = HistoryRecord::new(family, index, sample);
    rec.to_frame(family, &CodecOptions::default()).unwrap().encode(family).unwrap()
}

fn inbound_record(family: DeviceFamily, index: u32, ts_ms: i64, bpm: u8) -> Inbound {
    let raw = record_frame(family, index, ts_ms, bpm);
    let frame = unstrapped_core::protocol::Frame::decode(&raw, family).unwrap();
    Inbound { packet: Packet::decode(&frame, family).unwrap(), raw, characteristic: family.characteristic(unstrapped_core::protocol::CharacteristicRole::Data).unwrap().uuid.into() }
}

fn meta(m: HistoryMetadata, family: DeviceFamily) -> Inbound {
    let raw = Packet::HistoryMetadata(m).encode(family).unwrap();
    let frame = unstrapped_core::protocol::Frame::decode(&raw, family).unwrap();
    Inbound { packet: Packet::decode(&frame, family).unwrap(), raw, characteristic: "data".into() }
}

fn end_meta(token: &[u8], family: DeviceFamily) -> Inbound {
    meta(HistoryMetadata::End { seq: 1, end: HistoryEnd { unix: 1_800_000_000, subsec: 0, reserved: 0, ack_token: token.to_vec() } }, family)
}

// ------------------------------------------------------------------------------ DB

#[test]
fn migrations_create_schema_and_are_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("strap.sqlite3");
    let db = Db::open(&path).unwrap();
    assert_eq!(db.schema_version(), 1);
    for t in ["raw_frames", "heart_rate", "rr_intervals", "temperature", "motion", "spo2_raw", "steps", "sync_state", "daily_summary", "sleeps", "sleep_stages", "activities", "stress", "baselines", "journal", "settings", "assessments"] {
        assert_eq!(db.count(t), 0, "table {t} missing");
    }
    let mode: String = db.conn().query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
    assert_eq!(mode.to_lowercase(), "wal");
    db.save_settings(&Settings::default()).unwrap();
    drop(db);
    let db = Db::open(&path).unwrap(); // reopen: no re-migration, data kept
    assert_eq!(db.schema_version(), 1);
    assert_eq!(db.settings(), Settings::default());
}

#[test]
fn persist_chunk_dedupes_and_rebuild_restores_decoded_rows() {
    let db = Db::open_memory().unwrap();
    let fam = DeviceFamily::Whoop4;
    let base = now_ms() - 3_600_000;
    let frames: Vec<Vec<u8>> = (0..10).map(|i| record_frame(fam, i, base + i as i64 * 1000, 60 + i as u8)).collect();
    let end = HistoryEnd { unix: 1, subsec: 0, reserved: 0, ack_token: vec![1, 2, 3, 4, 5, 6, 7, 8] };
    let s1 = db.persist_history_chunk("S1", fam, "data", now_ms(), &frames, &end).unwrap();
    assert_eq!((s1.new_frames, s1.records), (10, 10));
    assert_eq!(db.count("raw_frames"), 10);
    assert_eq!(db.count("heart_rate"), 10);
    assert_eq!(db.count("rr_intervals"), 20);
    assert_eq!(db.count("temperature"), 10);
    assert_eq!(db.get_state("cursor_token").unwrap(), "0102030405060708");
    // an unacked chunk replays: nothing is duplicated
    let s2 = db.persist_history_chunk("S1", fam, "data", now_ms(), &frames, &end).unwrap();
    assert_eq!(s2.new_frames, 0);
    assert_eq!(db.count("raw_frames"), 10);
    assert_eq!(db.count("heart_rate"), 10);
    assert_eq!(db.count("rr_intervals"), 20);
    // decoded rows reference raw frames
    let orphans: i64 = db.conn().query_row("SELECT COUNT(*) FROM heart_rate WHERE frame_id NOT IN (SELECT id FROM raw_frames)", [], |r| r.get(0)).unwrap();
    assert_eq!(orphans, 0);
    // rebuild: wipe decoded rows, re-decode from raw
    db.conn().execute_batch("DELETE FROM heart_rate; DELETE FROM rr_intervals; DELETE FROM temperature;").unwrap();
    let (min, max) = db.rebuild_decoded().unwrap().unwrap();
    assert_eq!(max - min, 9000);
    assert_eq!(db.count("heart_rate"), 10);
    assert_eq!(db.count("rr_intervals"), 20);
    assert_eq!(db.count("raw_frames"), 10); // raw frames untouched
}

#[test]
fn implausible_timestamps_are_archived_but_not_decoded() {
    let db = Db::open_memory().unwrap();
    let fam = DeviceFamily::Whoop4;
    let frames = vec![record_frame(fam, 0, 1_262_044_800_000, 60)]; // 2010: unset strap clock
    let end = HistoryEnd { unix: 1, subsec: 0, reserved: 0, ack_token: vec![0; 8] };
    let s = db.persist_history_chunk("S1", fam, "data", now_ms(), &frames, &end).unwrap();
    assert_eq!((s.new_frames, s.records), (1, 0));
    assert_eq!(db.count("heart_rate"), 0);
    assert!(decode_history_frame(fam, &frames[0]).is_some());
}

// ------------------------------------------------------------------------------ persist-before-ack

struct MockTransport {
    writes: Arc<Mutex<Vec<Vec<u8>>>>,
    db: Db,
    rows_at_ack: Arc<Mutex<Vec<i64>>>,
    family: DeviceFamily,
}

#[async_trait]
impl StrapTransport for MockTransport {
    fn kind(&self) -> &'static str {
        "emulator"
    }
    async fn connect(&mut self, _p: &(dyn Fn(ConnPhase) + Send + Sync)) -> TResult<ConnectInfo> {
        unreachable!()
    }
    fn take_events(&mut self) -> Option<mpsc::Receiver<TransportEvent>> {
        None
    }
    async fn write(&mut self, _c: &str, bytes: &[u8]) -> TResult<()> {
        let frame = unstrapped_core::protocol::Frame::decode(bytes, self.family).unwrap();
        if frame.cmd == command_number::HISTORICAL_DATA_RESULT {
            let n: i64 = self.db.conn().query_row("SELECT COUNT(*) FROM heart_rate", [], |r| r.get(0)).unwrap_or(-1);
            self.rows_at_ack.lock().unwrap().push(n);
        }
        self.writes.lock().unwrap().push(bytes.to_vec());
        Ok(())
    }
    async fn disconnect(&mut self) {}
}

type Writes = Arc<Mutex<Vec<Vec<u8>>>>;

fn mock_link(db: &Db, family: DeviceFamily) -> (Link, Writes, Arc<Mutex<Vec<i64>>>) {
    let writes = Arc::new(Mutex::new(vec![]));
    let rows = Arc::new(Mutex::new(vec![]));
    let t = MockTransport { writes: writes.clone(), db: db.clone(), rows_at_ack: rows.clone(), family };
    (Link::new(family, "S1".into(), Box::new(t)), writes, rows)
}

fn fast_opts() -> SyncOptions {
    SyncOptions { idle_timeout: Duration::from_millis(600), request_timeout: Duration::from_millis(30) }
}

fn cmds(writes: &Arc<Mutex<Vec<Vec<u8>>>>, family: DeviceFamily) -> Vec<u8> {
    writes.lock().unwrap().iter().map(|w| unstrapped_core::protocol::Frame::decode(w, family).unwrap().cmd).collect()
}

#[tokio::test]
async fn ack_is_sent_only_after_the_chunk_is_committed() {
    let fam = DeviceFamily::Whoop4;
    let db = Db::open_memory().unwrap();
    let (link, writes, rows_at_ack) = mock_link(&db, fam);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let base = now_ms() - 600_000;
    tx.send(meta(HistoryMetadata::Start { seq: 1, payload: vec![] }, fam)).unwrap();
    for i in 0..5 {
        tx.send(inbound_record(fam, i, base + i as i64 * 1000, 70)).unwrap();
    }
    tx.send(end_meta(&[9; 8], fam)).unwrap();
    for i in 5..8 {
        tx.send(inbound_record(fam, i, base + i as i64 * 1000, 71)).unwrap();
    }
    tx.send(end_meta(&[8; 8], fam)).unwrap();
    tx.send(meta(HistoryMetadata::Complete { seq: 1, payload: vec![] }, fam)).unwrap();
    let em = CollectEmitter::default();
    let out = run_history_sync(&link, &mut rx, &db, &em, &fast_opts()).await.unwrap();
    assert!(out.completed);
    assert_eq!((out.chunks, out.records), (2, 8));
    // at the moment each ack hit the wire, that chunk's rows were already committed
    assert_eq!(*rows_at_ack.lock().unwrap(), vec![5, 8]);
    let c = cmds(&writes, fam);
    assert_eq!(c, vec![command_number::GET_DATA_RANGE, command_number::SEND_HISTORICAL_DATA, command_number::HISTORICAL_DATA_RESULT, command_number::HISTORICAL_DATA_RESULT]);
    assert_eq!(db.get_state("cursor_token").unwrap(), "0808080808080808");
}

#[tokio::test]
async fn failed_persist_never_acks_and_aborts() {
    let fam = DeviceFamily::Whoop4;
    let db = Db::open_memory().unwrap();
    db.conn().execute_batch("DROP TABLE rr_intervals;").unwrap(); // makes the chunk transaction fail
    let (link, writes, _) = mock_link(&db, fam);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let base = now_ms() - 600_000;
    tx.send(meta(HistoryMetadata::Start { seq: 1, payload: vec![] }, fam)).unwrap();
    tx.send(inbound_record(fam, 0, base, 70)).unwrap();
    tx.send(end_meta(&[7; 8], fam)).unwrap();
    let err = run_history_sync(&link, &mut rx, &db, &CollectEmitter::default(), &fast_opts()).await.unwrap_err();
    assert!(err.contains("Could not save"), "{err}");
    let c = cmds(&writes, fam);
    assert!(!c.contains(&command_number::HISTORICAL_DATA_RESULT), "acked a chunk that was not stored: {c:?}");
    assert_eq!(c.last(), Some(&command_number::ABORT_HISTORICAL_TRANSMITS));
    assert_eq!(db.count("raw_frames"), 0); // the transaction rolled back as a whole
    assert!(db.get_state("cursor_token").is_none());
}

#[tokio::test]
async fn idle_watchdog_aborts_a_silent_strap() {
    let fam = DeviceFamily::Whoop4;
    let db = Db::open_memory().unwrap();
    let (link, writes, _) = mock_link(&db, fam);
    let (_tx, mut rx) = mpsc::unbounded_channel::<Inbound>();
    let err = run_history_sync(&link, &mut rx, &db, &CollectEmitter::default(), &SyncOptions { idle_timeout: Duration::from_millis(700), request_timeout: Duration::from_millis(20) }).await.unwrap_err();
    assert!(err.contains("stopped sending"), "{err}");
    assert_eq!(cmds(&writes, fam).last(), Some(&command_number::ABORT_HISTORICAL_TRANSMITS));
    let _ = HostCommand::GetDataRange;
}

// ------------------------------------------------------------------------------ full flow vs the emulator

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_sync_against_in_process_emulator() {
    use crate::transport::inproc::InProcTransport;
    use unstrapped_emulator::{EmulatedStrap, EmulatorConfig, StrapOptions};

    let db = Db::open_memory().unwrap();
    let em = Arc::new(CollectEmitter::default());
    let factory: crate::strap::TransportFactory = Arc::new(|_, _, _| {
        let cfg = EmulatorConfig { history_days: 3, ..Default::default() };
        let opts = StrapOptions { record_interval_s: 15, history_records_per_sec: None, ..Default::default() };
        Box::new(InProcTransport::new(EmulatedStrap::with_options(cfg, opts)))
    });
    let dyn_em: Arc<dyn Emitter> = em.clone();
    let mgr = StrapManager::new(db.clone(), dyn_em, factory);
    mgr.connect("emulator").await.unwrap();
    assert_eq!(mgr.status().state, "connected");
    mgr.sync_now().await.unwrap();

    assert!(db.count("raw_frames") > 1000, "raw frames: {}", db.count("raw_frames"));
    assert!(db.count("heart_rate") > 1000);
    assert!(db.count("rr_intervals") > 1000);
    assert!(db.last_sync_ms().is_some());
    assert_eq!(mgr.status().state, "connected");
    // progress events: downloading with a label, ends idle
    let prog = em.named("sync://progress");
    assert!(prog.iter().any(|p| p["phase"] == "downloading" && p["label"].as_str().unwrap().starts_with("Downloading")));
    assert_eq!(prog.last().unwrap()["phase"], "idle");
    // the UI header listens to strap://status: its last event must carry the new sync time
    let st = em.named("strap://status");
    assert_eq!(st.last().unwrap()["lastSyncMs"].as_i64(), db.last_sync_ms(), "status event still says never synced");
    // scores were computed and announced
    let upd = em.named("data://updated");
    assert!(!upd.is_empty() && !upd.last().unwrap()["days"].as_array().unwrap().is_empty());
    assert!(db.count("daily_summary") >= 2);
    let today = crate::compute::today_string();
    let home = crate::screens::get_home(&db, &today).unwrap();
    let v = serde_json::to_value(&home).unwrap();
    assert!(["ok", "calibrating", "noData"].contains(&v["recovery"]["state"].as_str().unwrap()));

    // every screen builds and serializes for every computed day
    let days: Vec<String> = db.summaries_between("2000-01-01", "2100-01-01").unwrap().into_iter().map(|s| s.day).collect();
    for d in &days {
        use crate::{reports as rp, screens as sc};
        let home = serde_json::to_value(sc::get_home(&db, d).unwrap()).unwrap();
        eprintln!("{d}: recovery={} sleep={} strain={}", home["recovery"]["state"], home["sleep"]["state"], home["strain"]["state"]);
        serde_json::to_value(sc::get_sleep(&db, d).unwrap()).unwrap();
        serde_json::to_value(sc::get_recovery(&db, d).unwrap()).unwrap();
        serde_json::to_value(sc::get_strain(&db, d).unwrap()).unwrap();
        serde_json::to_value(sc::get_stress(&db, d).unwrap()).unwrap();
        serde_json::to_value(sc::get_health(&db, d, None).unwrap()).unwrap();
        for m in ["respRate", "spo2", "restingHr", "hrv", "skinTemp"] {
            sc::get_metric_detail(&db, m, d).unwrap();
        }
        let raw = rp::get_raw_day(&db, d).unwrap();
        assert_eq!(raw.hr.ts.len(), raw.hr.v.len());
        rp::get_raw_night(&db, d).unwrap();
    }
    for m in ["sleepPerformance", "hoursVsNeed", "timeInBed", "consistency", "restorativePct", "restorativeHours", "efficiency", "sleepDebt", "dayStrain", "strengthTime", "calories", "avgHr", "steps", "zones123", "zones45", "vo2max", "recovery", "restingHr", "hrv", "respRate", "stress"] {
        for r in ["1W", "1M", "6M", "1Y"] {
            serde_json::to_value(crate::reports::get_trend(&db, m, r).unwrap()).unwrap();
        }
    }
    crate::reports::get_healthspan(&db).unwrap();
    crate::reports::get_raw_trends(&db, "1M").unwrap();
    assert!(db.count("sleeps") >= 1, "no sleep detected in 3 emulated days");

    // a second sync with nothing new succeeds and adds nothing
    let before = db.count("heart_rate");
    mgr.sync_now().await.unwrap();
    assert_eq!(db.count("heart_rate"), before);

    // rebuild reproduces the decoded rows from raw frames
    mgr.rebuild().await.unwrap();
    assert_eq!(db.count("heart_rate"), before);
    mgr.disconnect().await.unwrap();
    assert_eq!(mgr.status().state, "disconnected");
}

// ------------------------------------------------------------------------------ JSON shapes

#[test]
fn avail_serializes_with_state_tags() {
    let ok = serde_json::to_value(Avail::ok(SleepPerfOnly { performance: 85.0 })).unwrap();
    assert_eq!(ok, json!({"state": "ok", "data": {"performance": 85.0}}));
    let cal: Avail<SleepPerfOnly> = Avail::Calibrating { nights_remaining: 2, message: None };
    assert_eq!(serde_json::to_value(cal).unwrap(), json!({"state": "calibrating", "nightsRemaining": 2}));
    let nd: Avail<SleepPerfOnly> = Avail::no_data_gaps("Strap was off", vec![Gap { start_ms: 1, end_ms: 2, reason: "Strap was off".into() }]);
    assert_eq!(serde_json::to_value(nd).unwrap(), json!({"state": "noData", "reason": "Strap was off", "gaps": [{"startMs": 1, "endMs": 2, "reason": "Strap was off"}]}));
}

#[test]
fn payloads_use_camel_case_keys() {
    let st = StrapStatus { transport: "ble".into(), state: "connected".into(), device_name: Some("W".into()), battery_pct: Some(80.0), charging: false, on_wrist: None, last_sync_ms: Some(5), last_hr: None, error: None };
    let v = serde_json::to_value(&st).unwrap();
    for k in ["transport", "state", "deviceName", "batteryPct", "charging", "onWrist", "lastSyncMs", "lastHr", "error"] {
        assert!(v.get(k).is_some(), "missing {k}");
    }
    let s = serde_json::to_value(Settings::default()).unwrap();
    assert_eq!(s["profile"]["weightKg"], 76.0);
    assert_eq!(s["profile"]["sex"], "male");
    assert_eq!(s["emulatorAddress"], "127.0.0.1:47800");
    assert!(s.get("autoSync").is_some() && s.get("launchAtLogin").is_some());
    let raw = serde_json::to_value(RawFrameEvent { received_at: 1, characteristic: "c".into(), hex: "aa".into() }).unwrap();
    assert!(raw.get("receivedAt").is_some());
    let a = serde_json::to_value(ActivitySummary { id: "x".into(), kind: "Run".into(), start_ms: 1, end_ms: 2, strain: 1.0, avg_hr: 1.0, max_hr: 2.0, calories: 3.0, zone_min: [1.0; 5], manual: true }).unwrap();
    assert!(a.get("zoneMin").unwrap().as_array().unwrap().len() == 5 && a.get("startMs").is_some());
    let p = serde_json::to_value(SyncProgress::new("downloading", 0.64, "Downloading 23 h of data… 64%")).unwrap();
    assert_eq!(p, json!({"phase": "downloading", "fraction": 0.64, "label": "Downloading 23 h of data… 64%"}));
}

fn sample_summary(day: &str, rec: f64) -> unstrapped_core::metrics::DailySummary {
    unstrapped_core::metrics::DailySummary {
        day: day.into(),
        recovery: Some(rec),
        recovery_band: Some("green".into()),
        sleep_performance: Some(80.0),
        day_strain: Some(10.0),
        hrv_ms: Some(60.0),
        resting_hr: Some(52.0),
        resp_rate: Some(14.5),
        calories: Some(2400.0),
        avg_hr: Some(70.0),
        steps: Some(8000),
        sleep_asleep_min: Some(450),
        sleep_in_bed_min: Some(490),
        sleep_start_ms: Some(1),
        sleep_end_ms: Some(2),
        ..Default::default()
    }
}

#[test]
fn screens_report_no_data_instead_of_zeros() {
    let db = Db::open_memory().unwrap();
    let v = serde_json::to_value(crate::screens::get_sleep(&db, "2026-01-01").unwrap()).unwrap();
    assert_eq!(v["state"], "noData");
    assert!(v["gaps"].is_array());
    let home = serde_json::to_value(crate::screens::get_home(&db, "2026-01-01").unwrap()).unwrap();
    for k in ["sleep", "recovery", "strain", "healthMonitor", "stress", "tonightsSleep", "keyStats"] {
        assert_eq!(home[k]["state"], "noData", "{k}");
    }
    assert!(home["activities"].as_array().unwrap().is_empty());
}

#[test]
fn trends_respect_direction_and_granularity() {
    let db = Db::open_memory().unwrap();
    let today = crate::compute::today_string();
    for i in 0..30 {
        let d = crate::compute::add_days(&today, -i);
        let mut s = sample_summary(&d, 50.0 + i as f64);
        s.resting_hr = Some(50.0 + i as f64);
        let res = unstrapped_core::metrics::DayResult {
            day: d.clone(),
            summary: s.clone(),
            sleeps: vec![],
            activities: vec![],
            stress: vec![],
            baselines: vec![],
            confidence: String::new(),
            recovery: unstrapped_core::metrics::ScoreResult::Unavailable { reason: "x".into() },
            sleep_performance: unstrapped_core::metrics::ScoreResult::Unavailable { reason: "x".into() },
            sleep_need: None,
            tonight: None,
            tonight_need: None,
            strain: None,
            stress_summary: unstrapped_core::metrics::stress::StressSummary { current: None, current_band: None, low_min: 0, medium_min: 0, high_min: 0, sleep_high_share: None },
            stress_provisional: false,
            health_monitor: unstrapped_core::metrics::monitor::HealthMonitor { metrics: vec![], in_range_count: 0, total_with_data: 0, early_warning: false, early_warning_signals: vec![], nights_of_baseline: 0 },
            baseline_set: Default::default(),
            calibration: unstrapped_core::metrics::Calibration::Solid { nights: 30 },
            gaps: vec![],
            notes: vec![],
        };
        let (a, b) = crate::compute::day_bounds(&d).unwrap();
        db.write_day_result(&res, a, b).unwrap();
    }
    let week = serde_json::to_value(crate::reports::get_trend(&db, "restingHr", "1W").unwrap()).unwrap();
    assert_eq!(week["state"], "ok");
    assert_eq!(week["data"]["granularity"], "daily");
    assert_eq!(week["data"]["points"].as_array().unwrap().len(), 7);
    // lower is better: best = lowest resting HR = today (50), worst = 6 days ago (56)
    assert_eq!(week["data"]["summary"]["best"]["value"], 50.0);
    assert_eq!(week["data"]["summary"]["worst"]["value"], 56.0);
    let half = serde_json::to_value(crate::reports::get_trend(&db, "recovery", "6M").unwrap()).unwrap();
    assert_eq!(half["data"]["granularity"], "weekly");
    assert!(half["data"]["points"][0]["band"].is_string());
    assert_eq!(serde_json::to_value(crate::reports::get_trend(&db, "hrv", "1W").unwrap()).unwrap()["state"], "ok");
    assert!(crate::reports::get_trend(&db, "nope", "1W").is_err());
}

#[test]
fn journal_roundtrip_and_insights_gate() {
    let db = Db::open_memory().unwrap();
    let lib = crate::reports::get_journal_library(&db);
    assert!(lib.iter().filter(|i| i.selected).count() == 6);
    db.set_journal_selected(&["alcohol".into(), "sauna".into()]).unwrap();
    db.set_journal_answer("2026-01-01", "alcohol", Some(true)).unwrap();
    let j = crate::reports::get_journal(&db, "2026-01-01");
    assert_eq!(j.questions.len(), 2);
    assert_eq!(j.questions[0].answer, Some(true));
    assert_eq!(j.questions[1].answer, None);
    db.set_journal_answer("2026-01-01", "alcohol", None).unwrap();
    assert_eq!(crate::reports::get_journal(&db, "2026-01-01").questions[0].answer, None);
    let ins = crate::reports::get_insights(&db);
    assert!(ins.iter().all(|i| i.state == "notEnough"));
}

#[test]
fn csv_export_writes_requested_rows() {
    let db = Db::open_memory().unwrap();
    let fam = DeviceFamily::Whoop4;
    let base = crate::compute::day_bounds(&crate::compute::today_string()).unwrap().0 + 1000;
    let frames: Vec<Vec<u8>> = (0..3).map(|i| record_frame(fam, i, base + i as i64 * 1000, 60)).collect();
    db.persist_history_chunk("S1", fam, "data", now_ms(), &frames, &HistoryEnd { unix: 1, subsec: 0, reserved: 0, ack_token: vec![0; 8] }).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("hr.csv");
    let t = crate::compute::today_string();
    let (a, b) = (crate::compute::day_bounds(&t).unwrap().0, crate::compute::day_bounds(&t).unwrap().1);
    let n = db.export_csv("heart_rate", a, b, &t, &t, &out).unwrap();
    assert_eq!(n, 3);
    let text = std::fs::read_to_string(&out).unwrap();
    assert!(text.starts_with("ts_ms,iso_time,bpm\n"));
    assert_eq!(text.lines().count(), 4);
}
