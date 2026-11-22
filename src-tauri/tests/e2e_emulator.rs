//! Real-path end-to-end: the app's `StrapManager` + `EmulatorTransport` over TCP against an in-process
//! `unstrapped_emulator` server (WHOOP 4 and WHOOP 5), temp DB, no GUI.
//! Ignored by default (about 5 min in debug, ~1 min in release):
//! `cargo test -p unstrapped --test e2e_emulator --release -- --ignored --nocapture` (prints the 7-day table).

use async_trait::async_trait;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use unstrapped_core::protocol::{DeviceFamily, Frame, command_number};
use unstrapped_emulator::server::{ServerOptions, serve_on};
use unstrapped_emulator::{EmulatorConfig, StrapOptions};
use unstrapped_lib::compute::{add_days, day_of, tz_offset_at};
use unstrapped_lib::db::Db;
use unstrapped_lib::emitter::CollectEmitter;
use unstrapped_lib::model::Settings;
use unstrapped_lib::strap::{StrapManager, TransportFactory};
use unstrapped_lib::transport::{ConnPhase, ConnectInfo, EmulatorTransport, StrapTransport, TResult, TransportEvent};
use unstrapped_lib::{reports, screens};

/// Wraps the real emulator transport and checks, at every history ack the app writes, that the chunk's
/// cursor token is already committed in SQLite (persist-before-ack).
struct SpyTransport {
    inner: EmulatorTransport,
    db: Db,
    family: Option<DeviceFamily>,
    acks: Arc<Mutex<Vec<(bool, i64)>>>, // (token committed?, raw_frames count at ack)
}

#[async_trait]
impl StrapTransport for SpyTransport {
    fn kind(&self) -> &'static str {
        "emulator"
    }
    async fn connect(&mut self, phase: &(dyn Fn(ConnPhase) + Send + Sync)) -> TResult<ConnectInfo> {
        let info = self.inner.connect(phase).await?;
        self.family = Some(info.family);
        Ok(info)
    }
    fn take_events(&mut self) -> Option<mpsc::Receiver<TransportEvent>> {
        self.inner.take_events()
    }
    async fn write(&mut self, characteristic: &str, bytes: &[u8]) -> TResult<()> {
        if let Some(fam) = self.family
            && let Ok(f) = Frame::decode(bytes, fam)
            && f.cmd == command_number::HISTORICAL_DATA_RESULT
        {
            let token = self.db.get_state("cursor_token").unwrap_or_default();
            let committed = !token.is_empty() && hex::encode(bytes).contains(&token);
            self.acks.lock().unwrap().push((committed, self.db.count("raw_frames")));
        }
        self.inner.write(characteristic, bytes).await
    }
    async fn disconnect(&mut self) {
        self.inner.disconnect().await
    }
    async fn control(&mut self, a: &str, v: Option<f64>) -> TResult<()> {
        self.inner.control(a, v).await
    }
}

async fn wait_for<F: FnMut() -> bool>(what: &str, secs: u64, mut f: F) {
    let t = Instant::now();
    while !f() {
        assert!(t.elapsed() < Duration::from_secs(secs), "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn state_of(v: &Value) -> String {
    v.get("state").and_then(|s| s.as_str()).unwrap_or("?").to_string()
}

fn hm(ms: i64) -> String {
    use chrono::{Local, TimeZone};
    Local.timestamp_millis_opt(ms).single().map(|d| d.format("%H:%M").to_string()).unwrap_or_default()
}

fn f(x: Option<f64>, d: usize) -> String {
    x.map(|v| format!("{v:.d$}")).unwrap_or_else(|| "-".into())
}

async fn run_family(family: DeviceFamily, seed: u64) {
    // ---- emulator on an ephemeral port
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let cfg = EmulatorConfig {
        family,
        seed,
        device_name: if family == DeviceFamily::Whoop4 { "WHOOP 4EMU0001".into() } else { "WHOOP 5EMU0001".into() },
        ..Default::default()
    };
    let opts = ServerOptions { fast_history: true, tick_ms: 2, strap: StrapOptions::default() };
    let server = tokio::spawn(async move {
        let _ = serve_on(listener, cfg, opts).await;
    });

    // ---- the app side
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(&dir.path().join("strap.sqlite3")).unwrap();
    db.save_settings(&Settings { transport: "emulator".into(), emulator_address: addr.clone(), ..Settings::default() }).unwrap();
    let em = Arc::new(CollectEmitter::default());
    let acks: Arc<Mutex<Vec<(bool, i64)>>> = Arc::new(Mutex::new(vec![]));
    let (db2, acks2) = (db.clone(), acks.clone());
    let factory: TransportFactory = Arc::new(move |_kind, settings: &Settings, _remembered| {
        Box::new(SpyTransport { inner: EmulatorTransport::new(settings.emulator_address.clone()), db: db2.clone(), family: None, acks: acks2.clone() })
    });
    let mgr = StrapManager::new(db.clone(), em.clone(), factory);

    // ---- connect: status events + live HR
    mgr.connect("emulator").await.expect("connect");
    let states: Vec<String> = em.named("strap://status").iter().map(state_of_status).collect();
    assert!(states.contains(&"scanning".to_string()) && states.contains(&"connecting".to_string()) && states.last().map(|s| s == "connected").unwrap_or(false), "status sequence: {states:?}");
    wait_for("live HR events", 15, || em.named("strap://hr").len() >= 3).await;
    wait_for("battery in status", 15, || mgr.status().battery_pct.is_some()).await;
    let hr = em.named("strap://hr")[0].clone();
    let bpm = hr["bpm"].as_u64().unwrap();
    assert!((30..=220).contains(&bpm), "live bpm {bpm}");
    assert!(!em.named("strap://raw").is_empty(), "raw frames are emitted");

    // ---- sync, interrupted midway by the emulator going out of range (through the app layer)
    let interrupter = {
        let (mgr, db) = (mgr.clone(), db.clone());
        tokio::spawn(async move {
            let t = Instant::now();
            while db.count("raw_frames") < 60_000 && t.elapsed() < Duration::from_secs(300) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            mgr.emulator_control("out_of_range", None).await.expect("out_of_range via app layer");
        })
    };
    let first = mgr.sync_now().await;
    interrupter.await.unwrap();
    assert!(first.is_err(), "sync must fail when the strap drops out of range");
    let mid_frames = db.count("raw_frames");
    assert!(mid_frames > 0, "data saved before the interruption is kept");
    assert!(db.last_sync_ms().is_none(), "an interrupted sync is not recorded as complete");
    wait_for("disconnected status", 15, || mgr.status().state == "disconnected").await;
    // out of range: connecting fails; in_range works with no connection (control-only client)
    assert!(mgr.connect("emulator").await.is_err(), "connect while out of range must fail");
    mgr.emulator_control("in_range", None).await.expect("in_range while out of range");
    mgr.connect("emulator").await.expect("reconnect");
    wait_for("connected", 15, || mgr.status().state == "connected").await;

    // ---- resume: finishes, nothing lost or duplicated
    mgr.sync_now().await.expect("resumed sync");
    assert!(db.last_sync_ms().is_some());
    let total = db.count("raw_frames");
    assert!(total > mid_frames);
    let hr_rows = db.count("heart_rate");
    assert!(hr_rows > 700_000, "14 days of 1 Hz HR expected, got {hr_rows}");
    for t in ["rr_intervals", "temperature", "motion", "daily_summary", "sleeps", "sleep_stages", "stress"] {
        assert!(db.count(t) > 0, "table {t} is empty");
    }
    if family == DeviceFamily::Whoop4 {
        assert!(db.count("spo2_raw") > 0, "W4 raw SpO2 rows");
    } else {
        assert!(db.count("steps") > 0, "W5 step rows");
    }
    // persist-before-ack on the real path
    let acks = acks.lock().unwrap().clone();
    assert!(acks.len() > 100, "acks observed: {}", acks.len());
    assert!(acks.iter().all(|(ok, _)| *ok), "an ack was sent before its cursor token was committed");
    // no duplicates (PK) and no gaps beyond strap-off windows (local 10:00-15:00)
    let distinct: i64 = db.conn().query_row("SELECT COUNT(DISTINCT ts_ms) FROM heart_rate", [], |r| r.get(0)).unwrap();
    assert_eq!(distinct, hr_rows, "duplicate heart_rate timestamps");
    let ts: Vec<i64> = {
        let c = db.conn();
        let mut st = c.prepare("SELECT ts_ms FROM heart_rate ORDER BY ts_ms").unwrap();
        st.query_map([], |r| r.get::<_, i64>(0)).unwrap().map(|r| r.unwrap()).collect()
    };
    let mut bad = vec![];
    for w in ts.windows(2) {
        if w[1] - w[0] > 2_000 {
            let off = tz_offset_at(w[0]) as i64 * 60_000;
            let (a, b) = (((w[0] + off) / 60_000).rem_euclid(1440), ((w[1] + off) / 60_000).rem_euclid(1440));
            if !(595..=905).contains(&a) || !(595..=905).contains(&b) || w[1] - w[0] > 60 * 60_000 {
                bad.push((hm(w[0]), hm(w[1]), (w[1] - w[0]) / 1000));
            }
        }
    }
    assert!(bad.is_empty(), "unexpected history gaps after resume: {bad:?}");
    let span_days = (ts[ts.len() - 1] - ts[0]) as f64 / 86_400_000.0;
    assert!(span_days > 12.5, "history spans {span_days:.1} days");

    // a second sync only transfers what is new
    mgr.sync_now().await.expect("second sync");
    assert!(db.count("raw_frames") - total < 5_000, "second sync should be incremental");

    // ---- scores for most days
    let today = day_of(chrono::Utc::now().timestamp_millis());
    let from = add_days(&today, -13);
    let sums = db.summaries_between(&from, &today).unwrap();
    let scored_rec = sums.iter().filter(|s| s.recovery.is_some()).count();
    let scored_strain = sums.iter().filter(|s| s.day_strain.is_some()).count();
    assert!(sums.len() >= 11, "daily summaries: {}", sums.len());
    assert!(scored_rec >= 7, "recovery scored on {scored_rec} of {} days (calibration should end)", sums.len());
    assert!(scored_strain >= 11, "strain on {scored_strain} days");

    // ---- screens for a late day return ok JSON
    let day = add_days(&today, -1);
    let j = |v: &dyn erased::Ser| v.json();
    let checks: Vec<(&str, Value)> = vec![
        ("get_sleep", j(&screens::get_sleep(&db, &day).unwrap())),
        ("get_recovery", j(&screens::get_recovery(&db, &day).unwrap())),
        ("get_strain", j(&screens::get_strain(&db, &day).unwrap())),
        ("get_stress", j(&screens::get_stress(&db, &day).unwrap())),
        ("get_trend", j(&reports::get_trend(&db, "recovery", "1W").unwrap())),
        ("get_trend1M", j(&reports::get_trend(&db, "hrv", "1M").unwrap())),
        ("get_metric_detail", j(&screens::get_metric_detail(&db, "hrv", &day).unwrap())),
    ];
    for (name, v) in &checks {
        assert_eq!(state_of(v), "ok", "{name} -> {v}");
    }
    let sleep = &checks[0].1["data"];
    for c in sleep["contributors"].as_array().unwrap() {
        println!("  sleep contributor {} pct={}", c["key"], c["pct"]);
    }
    let home = j(&screens::get_home(&db, &day).unwrap());
    for k in ["recovery", "sleep", "strain"] {
        if let Some(v) = home.get(k) {
            assert_eq!(state_of(v), "ok", "home.{k} -> {v}");
        }
    }
    let health = j(&screens::get_health(&db, &day, None).unwrap());
    assert!(health.is_object());
    let _insights = reports::get_insights(&db); // 'notEnough' is expected without journal data
    let assess = db.list_assessments().unwrap();
    println!("[{family:?}] assessments: {}", assess.len());
    let raw = reports::get_raw_day(&db, &day).unwrap();
    let raw = j(&raw);
    assert!(raw["hr"]["ts"].as_array().map(|a| a.len() > 600).unwrap_or(false), "raw day HR series too short: {raw:.200}");

    // ---- 7-day table
    println!("\n[{family:?}] last 7 days (local times)");
    println!("{:<11}{:>4} {:<7}{:>6}{:>7}{:>6}{:>5}{:>6}  sleep", "day", "rec", "band", "sleepP", "strain", "hrv", "rhr", "resp");
    let from7 = add_days(&today, -7);
    for s in db.summaries_between(&from7, &today).unwrap() {
        let sl: Option<(i64, i64)> = db
            .conn()
            .query_row("SELECT start_ms, end_ms FROM sleeps WHERE day=?1 AND is_nap=0 ORDER BY (end_ms-start_ms) DESC LIMIT 1", [&s.day], |r| Ok((r.get(0)?, r.get(1)?)))
            .ok();
        println!(
            "{:<11}{:>4} {:<7}{:>6}{:>7}{:>6}{:>5}{:>6}  {}",
            s.day,
            f(s.recovery, 0),
            s.recovery_band.clone().unwrap_or("-".into()),
            f(s.sleep_performance, 0),
            f(s.day_strain, 1),
            f(s.hrv_ms, 0),
            f(s.resting_hr, 0),
            f(s.resp_rate, 1),
            sl.map(|(a, b)| format!("{} -> {}", hm(a), hm(b))).unwrap_or("-".into())
        );
    }

    // ---- emulator controls through the app layer while connected
    mgr.emulator_control("take_off", None).await.unwrap();
    wait_for("wrist off", 10, || mgr.status().on_wrist == Some(false)).await;
    mgr.emulator_control("put_on", None).await.unwrap();
    wait_for("wrist on", 10, || mgr.status().on_wrist == Some(true)).await;
    mgr.emulator_control("advance_minutes", Some(5)).await.unwrap();
    let _ = mgr.emulator_control("bogus_action", None).await; // unknown actions answer with an error, never hang
    mgr.disconnect().await.unwrap();
    assert_eq!(mgr.status().state, "disconnected");
    server.abort();
}

fn state_of_status(v: &Value) -> String {
    v["state"].as_str().unwrap_or("?").to_string()
}

mod erased {
    use serde::Serialize;
    pub trait Ser {
        fn json(&self) -> serde_json::Value;
    }
    impl<T: Serialize> Ser for T {
        fn json(&self) -> serde_json::Value {
            serde_json::to_value(self).unwrap()
        }
    }
}

#[ignore = "slow: 14 days at 1 Hz over TCP; run with --release -- --ignored"]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn whoop4_end_to_end_over_tcp() {
    run_family(DeviceFamily::Whoop4, 42).await;
}

#[ignore = "slow: 14 days at 1 Hz over TCP; run with --release -- --ignored"]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn whoop5_end_to_end_over_tcp() {
    run_family(DeviceFamily::Whoop5, 7).await;
}
