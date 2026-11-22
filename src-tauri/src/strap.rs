//! Connection manager: owns the (single) strap session, decodes everything the strap sends, emits
//! `strap://raw|hr|status`, tracks battery / wrist state, runs syncs and reconnects.

use crate::db::Db;
use crate::emitter::{Emitter, emit};
use crate::model::*;
use crate::sync::{SyncOptions, now_ms, run_history_sync};
use crate::transport::{BleTransport, ConnPhase, ConnectInfo, EmulatorTransport, StrapTransport, TResult, TransportEvent};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use unstrapped_core::protocol::{
    CharacteristicRole, DeviceFamily, HostCommand, Packet, ReassemblerSet, Reassembled, ResponseBody, SeqCounter, StdHeartRate,
    event_number, gatt::STANDARD_HEART_RATE_MEASUREMENT,
};

/// A decoded packet plus the exact bytes it arrived as.
#[derive(Debug, Clone)]
pub struct Inbound {
    pub packet: Packet,
    pub raw: Vec<u8>,
    pub characteristic: String,
}

pub type AssessmentHook = Arc<dyn Fn(&AssessmentListItem) + Send + Sync>;

pub type Sink = Arc<Mutex<Option<mpsc::UnboundedSender<Inbound>>>>;

/// Write side of a connection.
pub struct Link {
    pub family: DeviceFamily,
    pub strap_id: String,
    pub data_char: String,
    transport: tokio::sync::Mutex<Box<dyn StrapTransport>>,
    seq: Mutex<SeqCounter>,
}

impl Link {
    pub fn new(family: DeviceFamily, strap_id: String, transport: Box<dyn StrapTransport>) -> Self {
        let data_char = family.characteristic(CharacteristicRole::Data).map(|c| c.uuid.to_string()).unwrap_or_default();
        Self { family, strap_id, data_char, transport: tokio::sync::Mutex::new(transport), seq: Mutex::new(SeqCounter::new(1)) }
    }

    pub async fn send(&self, cmd: &HostCommand) -> TResult<()> {
        let seq = self.seq.lock().unwrap().next_seq();
        let bytes = cmd.encode(self.family, seq).map_err(|e| format!("encode failed: {e:?}"))?;
        self.send_raw(&bytes).await
    }

    pub async fn send_raw(&self, bytes: &[u8]) -> TResult<()> {
        let mut t = self.transport.lock().await;
        t.write(self.family.command_characteristic_uuid(), bytes).await
    }

    pub async fn control(&self, action: &str, value: Option<f64>) -> TResult<()> {
        self.transport.lock().await.control(action, value).await
    }

    pub async fn close(&self) {
        self.transport.lock().await.disconnect().await;
    }
}

struct Session {
    link: Arc<Link>,
    sink: Sink,
    reader: tokio::task::JoinHandle<()>,
    kind: String,
}

pub type TransportFactory = Arc<dyn Fn(&str, &Settings, Option<String>) -> Box<dyn StrapTransport> + Send + Sync>;

pub fn default_factory() -> TransportFactory {
    Arc::new(|kind, settings, remembered| {
        if kind == "emulator" {
            Box::new(EmulatorTransport::new(settings.emulator_address.clone()))
        } else {
            Box::new(BleTransport::new(remembered, Duration::from_secs(20)))
        }
    })
}

#[derive(Default, Clone)]
pub struct Hooks {
    /// Called after a successful sync (last sync ms).
    pub on_synced: Option<Arc<dyn Fn(i64) + Send + Sync>>,
    pub on_assessment: Option<AssessmentHook>,
}

pub struct StrapManager {
    pub db: Db,
    em: Arc<dyn Emitter>,
    factory: TransportFactory,
    status: Mutex<StrapStatus>,
    session: tokio::sync::Mutex<Option<Session>>,
    op_lock: tokio::sync::Mutex<()>,
    sync_lock: tokio::sync::Mutex<()>,
    want_connected: AtomicBool,
    hooks: Mutex<Hooks>,
    pub sync_opts: Mutex<SyncOptions>,
}

impl StrapManager {
    pub fn new(db: Db, em: Arc<dyn Emitter>, factory: TransportFactory) -> Arc<Self> {
        let s = db.settings();
        let status = StrapStatus {
            transport: s.transport.clone(),
            state: "disconnected".into(),
            device_name: None,
            battery_pct: db.get_state("battery_pct").and_then(|v| v.parse().ok()),
            charging: false,
            on_wrist: None,
            last_sync_ms: db.last_sync_ms(),
            last_hr: None,
            error: None,
        };
        Arc::new(Self {
            db,
            em,
            factory,
            status: Mutex::new(status),
            session: tokio::sync::Mutex::new(None),
            op_lock: tokio::sync::Mutex::new(()),
            sync_lock: tokio::sync::Mutex::new(()),
            want_connected: AtomicBool::new(false),
            hooks: Mutex::new(Hooks::default()),
            sync_opts: Mutex::new(SyncOptions::default()),
        })
    }

    pub fn set_hooks(&self, h: Hooks) {
        *self.hooks.lock().unwrap() = h;
    }

    pub fn emitter(&self) -> &Arc<dyn Emitter> {
        &self.em
    }

    // -------------------------------------------------------------------------- status
    pub fn status(&self) -> StrapStatus {
        let mut s = self.status.lock().unwrap().clone();
        s.last_sync_ms = self.db.last_sync_ms();
        s
    }

    fn update(&self, f: impl FnOnce(&mut StrapStatus)) {
        let snapshot = {
            let mut s = self.status.lock().unwrap();
            let before = s.clone();
            f(&mut s);
            // Always report the stored last-sync time, never the value cached at startup.
            s.last_sync_ms = self.db.last_sync_ms();
            if *s == before {
                return;
            }
            s.clone()
        };
        emit(self.em.as_ref(), "strap://status", &snapshot);
    }

    pub async fn is_connected(&self) -> bool {
        self.session.lock().await.is_some()
    }

    pub fn live_hr(&self) -> Option<f64> {
        let s = self.status.lock().unwrap();
        if s.state == "connected" || s.state == "syncing" { s.last_hr } else { None }
    }

    // -------------------------------------------------------------------------- connect / disconnect
    pub async fn connect(self: &Arc<Self>, kind: &str) -> Result<(), String> {
        let _g = self.op_lock.lock().await;
        self.want_connected.store(true, Ordering::SeqCst);
        self.connect_locked(kind).await
    }

    async fn connect_locked(self: &Arc<Self>, kind: &str) -> Result<(), String> {
        {
            let guard = self.session.lock().await;
            if let Some(s) = guard.as_ref()
                && s.kind == kind {
                    return Ok(());
                }
        }
        self.teardown().await;
        let settings = self.db.settings();
        self.update(|s| {
            s.transport = kind.to_string();
            s.state = "scanning".into();
            s.error = None;
            s.on_wrist = None;
            s.last_hr = None;
        });
        let remembered = self.db.strap_id(kind);
        let mut transport = (self.factory)(kind, &settings, remembered);
        let me = self.clone();
        let phase = move |p: ConnPhase| {
            me.update(|s| s.state = if p == ConnPhase::Scanning { "scanning".into() } else { "connecting".into() });
        };
        let info = match transport.connect(&phase).await {
            Ok(i) => i,
            Err(e) => {
                transport.disconnect().await;
                self.update(|s| {
                    s.state = "error".into();
                    s.error = Some(e.clone());
                });
                return Err(e);
            }
        };
        let _ = self.db.remember_strap(kind, &info.device_id);
        let events = transport.take_events().ok_or("transport produced no event stream")?;
        let link = Arc::new(Link::new(info.family, info.device_id.clone(), transport));
        let sink: Sink = Arc::new(Mutex::new(None));
        let reader = tokio::spawn(Self::reader_loop(self.clone(), link.clone(), events, sink.clone(), kind.to_string()));
        *self.session.lock().await = Some(Session { link: link.clone(), sink, reader, kind: kind.to_string() });

        // handshake (once per connection; never re-run mid-offload)
        if let Err(e) = self.handshake(&link, &info).await {
            self.teardown().await;
            self.update(|s| {
                s.state = "error".into();
                s.error = Some(e.clone());
            });
            return Err(e);
        }
        self.update(|s| {
            s.state = "connected".into();
            s.device_name = Some(info.name.clone());
            s.error = None;
        });
        Ok(())
    }

    async fn handshake(&self, link: &Link, info: &ConnectInfo) -> TResult<()> {
        if let Some(hello) = info.family.client_hello() {
            link.send_raw(hello).await?;
        }
        link.send(&HostCommand::GetBatteryLevel).await?;
        // live heart rate (best effort; the standard 0x2A37 service also feeds it)
        let _ = link.send(&HostCommand::ToggleRealtimeHr(true)).await;
        Ok(())
    }

    /// Drop the session without touching `want_connected`.
    async fn teardown(&self) {
        let sess = self.session.lock().await.take();
        if let Some(s) = sess {
            s.reader.abort();
            *s.sink.lock().unwrap() = None;
            s.link.close().await;
        }
    }

    pub async fn disconnect(&self) -> Result<(), String> {
        self.want_connected.store(false, Ordering::SeqCst);
        let _g = self.op_lock.lock().await;
        self.teardown().await;
        self.update(|s| {
            s.state = "disconnected".into();
            s.error = None;
            s.last_hr = None;
            s.on_wrist = None;
        });
        Ok(())
    }

    pub async fn emulator_control(&self, action: &str, value: Option<i64>) -> Result<(), String> {
        let link = self.session.lock().await.as_ref().map(|s| s.link.clone());
        if let Some(link) = link {
            return link.control(action, value.map(|v| v as f64)).await;
        }
        // Not connected (e.g. the strap is out of range): open a control-only connection, send, drop it.
        let addr = self.db.settings().emulator_address;
        let (mut c, _) = unstrapped_emulator::client::EmulatorClient::connect_any(&addr)
            .await
            .map_err(|e| format!("Could not reach the strap emulator at {addr} ({e})."))?;
        c.control(action, value.map(|v| v as f64)).await.map_err(|e| e.to_string())
    }

    // -------------------------------------------------------------------------- reader
    async fn reader_loop(me: Arc<Self>, link: Arc<Link>, mut events: mpsc::Receiver<TransportEvent>, sink: Sink, kind: String) {
        let family = link.family;
        let mut set = ReassemblerSet::new(family);
        let mut live: Vec<(String, i64, Vec<u8>)> = Vec::new();
        let mut flush = tokio::time::interval(Duration::from_secs(10));
        let mut throttle = Throttle::new(20);
        let mut reason = "Strap disconnected".to_string();
        loop {
            tokio::select! {
                _ = flush.tick() => me.flush_live(&link, &mut live),
                ev = events.recv() => {
                    let Some(ev) = ev else { break };
                    match ev {
                        TransportEvent::Disconnected(r) => { reason = format!("Strap disconnected ({r})"); break; }
                        TransportEvent::Notify { characteristic, bytes } => {
                            let received_at = now_ms();
                            if throttle.allow() {
                                emit(me.em.as_ref(), "strap://raw", &RawFrameEvent { received_at, characteristic: characteristic.clone(), hex: hex::encode(&bytes) });
                            }
                            if characteristic.eq_ignore_ascii_case(STANDARD_HEART_RATE_MEASUREMENT) {
                                if let Ok(hr) = StdHeartRate::decode(&bytes) {
                                    me.live_hr_sample(received_at, hr.bpm);
                                }
                                continue;
                            }
                            let Some(items) = set.push_uuid(&characteristic, &bytes) else { continue };
                            for item in items {
                                let Reassembled::Frame { frame, raw } = item else { continue };
                                let Ok(packet) = Packet::decode(&frame, family) else { continue };
                                if matches!(packet, Packet::RealtimeHr(_)) {
                                    live.push((characteristic.clone(), received_at, raw.clone()));
                                    if live.len() >= 60 { me.flush_live(&link, &mut live); }
                                }
                                me.handle_packet(&packet, received_at, family);
                                if let Some(tx) = sink.lock().unwrap().as_ref() {
                                    let _ = tx.send(Inbound { packet, raw, characteristic: characteristic.clone() });
                                }
                            }
                        }
                    }
                }
            }
        }
        me.flush_live(&link, &mut live);
        // Unexpected loss (an intentional disconnect aborts this task before we get here).
        *sink.lock().unwrap() = None; // closes the sync channel
        let still_current = {
            let mut g = me.session.lock().await;
            if g.as_ref().is_some_and(|s| Arc::ptr_eq(&s.link, &link)) {
                g.take();
                true
            } else {
                false
            }
        };
        if still_current {
            link.close().await;
            me.update(|s| {
                s.state = "disconnected".into();
                s.error = Some(reason.clone());
                s.last_hr = None;
                s.on_wrist = None;
            });
            if me.want_connected.load(Ordering::SeqCst) {
                tokio::spawn(Self::reconnect(me.clone(), kind));
            }
        }
    }

    fn reconnect(me: Arc<Self>, kind: String) -> futures::future::BoxFuture<'static, ()> {
        Box::pin(async move {
        for delay in [5u64, 20, 60] {
            tokio::time::sleep(Duration::from_secs(delay)).await;
            if !me.want_connected.load(Ordering::SeqCst) {
                return;
            }
            let _g = me.op_lock.lock().await;
            if !me.want_connected.load(Ordering::SeqCst) || me.session.lock().await.is_some() {
                return;
            }
            if me.connect_locked(&kind).await.is_ok() {
                return;
            }
        }
        })
    }

    fn flush_live(&self, link: &Link, live: &mut Vec<(String, i64, Vec<u8>)>) {
        if live.is_empty() {
            return;
        }
        let frames = std::mem::take(live);
        if let Err(e) = self.db.insert_live_frames(&link.strap_id, link.family, &frames) {
            eprintln!("could not store live frames: {e}");
        }
    }

    fn live_hr_sample(&self, ts: i64, bpm: u16) {
        if bpm == 0 {
            return;
        }
        emit(self.em.as_ref(), "strap://hr", &HrEvent { ts, bpm });
        self.update(|s| s.last_hr = Some(bpm as f64));
    }

    fn handle_packet(&self, packet: &Packet, received_at: i64, family: DeviceFamily) {
        match packet {
            Packet::RealtimeHr(hr) => self.live_hr_sample(received_at, hr.bpm as u16),
            Packet::Event(ev) => {
                if let Some(b) = ev.as_battery() {
                    self.set_battery(b.percent as f64, Some(b.charging));
                } else {
                    match ev.number {
                        event_number::WRIST_ON => self.update(|s| s.on_wrist = Some(true)),
                        event_number::WRIST_OFF => self.update(|s| {
                            s.on_wrist = Some(false);
                            s.last_hr = None;
                        }),
                        event_number::CHARGING_ON => self.update(|s| s.charging = true),
                        event_number::CHARGING_OFF => self.update(|s| s.charging = false),
                        _ => {}
                    }
                }
            }
            Packet::CommandResponse(r) => {
                if r.cmd == unstrapped_core::protocol::command_number::GET_BATTERY_LEVEL
                    && let Ok(ResponseBody::Battery { percent }) = r.typed_body(family) {
                        self.set_battery(percent as f64, None);
                    }
            }
            _ => {}
        }
    }

    fn set_battery(&self, pct: f64, charging: Option<bool>) {
        let pct = (pct * 10.0).round() / 10.0;
        let _ = self.db.set_state("battery_pct", &pct.to_string());
        self.update(|s| {
            s.battery_pct = Some(pct);
            if let Some(c) = charging {
                s.charging = c;
            }
        });
    }

    // -------------------------------------------------------------------------- sync
    pub async fn sync_now(self: &Arc<Self>) -> Result<(), String> {
        let _guard = self.sync_lock.try_lock().map_err(|_| "A sync is already running.".to_string())?;
        let (link, sink) = {
            let g = self.session.lock().await;
            let s = g.as_ref().ok_or("The strap is not connected.")?;
            (s.link.clone(), s.sink.clone())
        };
        let (tx, mut rx) = mpsc::unbounded_channel();
        *sink.lock().unwrap() = Some(tx);
        self.update(|s| s.state = "syncing".into());
        let opts = self.sync_opts.lock().unwrap().clone();
        let result = run_history_sync(&link, &mut rx, &self.db, self.em.as_ref(), &opts).await;
        *sink.lock().unwrap() = None;
        drop(rx);
        let still_connected = self.session.lock().await.is_some();
        let back_to = if still_connected { "connected" } else { "disconnected" };

        match result {
            Err(e) => {
                emit(self.em.as_ref(), "sync://progress", &SyncProgress::new("error", 0.0, e.clone()));
                self.update(|s| {
                    s.state = back_to.into();
                    s.error = Some(e.clone());
                });
                emit(self.em.as_ref(), "sync://progress", &SyncProgress::idle());
                Err(e)
            }
            Ok(outcome) => {
                if outcome.completed {
                    let _ = self.db.set_state("last_sync_ms", &now_ms().to_string());
                }
                emit(self.em.as_ref(), "sync://progress", &SyncProgress::new("processing", 0.99, "Calculating scores…"));
                let db = self.db.clone();
                let em = self.em.clone();
                let range = outcome.min_ts.zip(outcome.max_ts);
                let computed = tokio::task::spawn_blocking(move || crate::compute::process_new_data(&db, em.as_ref(), range)).await.map_err(|e| e.to_string());
                self.update(|s| {
                    s.state = back_to.into();
                    s.error = None;
                });
                let hooks = self.hooks.lock().unwrap().clone();
                match computed {
                    Ok(Ok(c)) => {
                        emit(self.em.as_ref(), "data://updated", &DataUpdatedEvent { days: c.days });
                        if let Some(h) = &hooks.on_assessment {
                            for a in &c.new_assessments {
                                h(a);
                            }
                        }
                    }
                    Ok(Err(e)) | Err(e) => {
                        emit(self.em.as_ref(), "sync://progress", &SyncProgress::new("error", 0.0, format!("Synced, but scores could not be calculated: {e}")));
                    }
                }
                emit(self.em.as_ref(), "sync://progress", &SyncProgress::new("done", 1.0, "Synced"));
                if let Some(h) = &hooks.on_synced {
                    h(self.db.last_sync_ms().unwrap_or_else(now_ms));
                }
                emit(self.em.as_ref(), "sync://progress", &SyncProgress::idle());
                Ok(())
            }
        }
    }

    /// Re-decode every stored frame, recompute every day. Emits `processing` progress then `data://updated`.
    pub async fn rebuild(self: &Arc<Self>) -> Result<(), String> {
        emit(self.em.as_ref(), "sync://progress", &SyncProgress::new("processing", 0.1, "Re-decoding stored data…"));
        let db = self.db.clone();
        let em = self.em.clone();
        let res = tokio::task::spawn_blocking(move || crate::compute::rebuild_all(&db, em.as_ref())).await.map_err(|e| e.to_string())?;
        match res {
            Ok(days) => {
                emit(self.em.as_ref(), "data://updated", &DataUpdatedEvent { days });
                emit(self.em.as_ref(), "sync://progress", &SyncProgress::new("done", 1.0, "Rebuilt"));
                emit(self.em.as_ref(), "sync://progress", &SyncProgress::idle());
                Ok(())
            }
            Err(e) => {
                emit(self.em.as_ref(), "sync://progress", &SyncProgress::new("error", 0.0, e.clone()));
                emit(self.em.as_ref(), "sync://progress", &SyncProgress::idle());
                Err(e)
            }
        }
    }

    /// Recompute every stored day from the decoded data (profile changed); no re-decoding.
    pub async fn rescore(self: &Arc<Self>) -> Result<(), String> {
        let db = self.db.clone();
        let em = self.em.clone();
        let days = tokio::task::spawn_blocking(move || crate::compute::rescore_all(&db, em.as_ref())).await.map_err(|e| e.to_string())??;
        emit(self.em.as_ref(), "data://updated", &DataUpdatedEvent { days });
        emit(self.em.as_ref(), "sync://progress", &SyncProgress::idle());
        Ok(())
    }

    pub fn is_syncing(&self) -> bool {
        self.sync_lock.try_lock().is_err()
    }
}

/// Simple per-second event limiter.
struct Throttle {
    limit: u32,
    window: Instant,
    count: u32,
}

impl Throttle {
    fn new(limit: u32) -> Self {
        Self { limit, window: Instant::now(), count: 0 }
    }
    fn allow(&mut self) -> bool {
        if self.window.elapsed() >= Duration::from_secs(1) {
            self.window = Instant::now();
            self.count = 0;
        }
        if self.count < self.limit {
            self.count += 1;
            true
        } else {
            false
        }
    }
}
