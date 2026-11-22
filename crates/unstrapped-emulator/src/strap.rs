//! The emulated strap: deterministic, synchronous, driven by a caller-supplied clock.

use std::collections::{HashSet, VecDeque};

use unstrapped_core::protocol::gatt::{STANDARD_BATTERY_LEVEL, STANDARD_HEART_RATE_MEASUREMENT};
use unstrapped_core::protocol::{
    BatteryEvent, CharacteristicRole, ClockReading, CodecOptions, CommandResponse, CommandResult, DataRange,
    Event, Frame, HistoryEnd, HistoryMetadata, HistoryRecord, HistorySample, HostCommand, Motion, Packet,
    Reassembled, Reassembler, RealtimeHr, ResponseBody, Spo2Raw, StdHeartRate, VersionInfo, event_number, fragment,
};

use crate::physio::{Activity, Physiology};
use crate::{CharacteristicInfo, DeviceFamily, EmulatorConfig, Notification};

/// Extra knobs beyond [`EmulatorConfig`] (so `EmulatorConfig` literals keep compiling).
#[derive(Debug, Clone)]
pub struct StrapOptions {
    /// Seconds between stored history records (real strap: 1).
    pub record_interval_s: u32,
    /// ATT MTU used to fragment notifications (each carries `mtu - 3` bytes).
    pub att_mtu: usize,
    /// History records per chunk (metadata END / ack cycle). NOOP: about 50.
    pub chunk_records: usize,
    /// Throttle for history streaming, records per real second. `None` = as fast as the host polls.
    pub history_records_per_sec: Option<f64>,
    /// Local timezone offset (minutes east of UTC) for bed/wake times. Default: the system's.
    pub tz_offset_min: i32,
}

impl Default for StrapOptions {
    fn default() -> Self {
        StrapOptions {
            record_interval_s: 1,
            att_mtu: 23,
            chunk_records: 50,
            history_records_per_sec: Some(150.0),
            tz_offset_min: crate::local_utc_offset_min(),
        }
    }
}

const CAPACITY_PAGES: u32 = 131_072;
const RECORDS_PER_PAGE: i64 = 15;
const DRAIN_PCT_PER_S: f64 = 100.0 / (4.5 * 86_400.0);
const CHARGE_PCT_PER_S: f64 = 100.0 / (75.0 * 60.0);

enum Phase {
    SendStart,
    NeedChunk,
    Waiting { end_idx: i64 },
    Done,
}

struct SyncRun {
    next_idx: i64,
    phase: Phase,
}

struct HistItem {
    notifs: Vec<Notification>,
    cost: f64,
}

pub struct EmulatedStrap {
    family: DeviceFamily,
    name: String,
    id: String,
    opts: StrapOptions,
    time_scale: f64,
    phys: Physiology,
    subs: HashSet<String>,
    connected: bool,
    rx: Reassembler,
    outbox: VecDeque<Notification>,
    now_ms: i64,
    last_tick_ms: i64,
    clock_offset_s: i64,
    seq: u8,
    realtime_on: bool,
    last_rt_sec: i64,
    battery: f64,
    charging: bool,
    last_batt_pct: i64,
    gaps: Vec<(i64, i64)>,
    wrist_prev: bool,
    trim_idx: Option<i64>,
    sync: Option<SyncRun>,
    hist_q: VecDeque<HistItem>,
    credit: f64,
    history_days: i64,
}

fn mk_id(seed: u64, family: DeviceFamily) -> String {
    let h = crate::rng::splitmix(seed ^ 0x57_48_4F_4F_50);
    let tag = if family == DeviceFamily::Whoop4 { "4" } else { "5" };
    format!("EMU-{tag}-{:08X}", (h & 0xFFFF_FFFF) as u32)
}

impl EmulatedStrap {
    pub fn new(config: EmulatorConfig) -> Self {
        Self::with_options(config, StrapOptions::default())
    }

    pub fn with_options(config: EmulatorConfig, opts: StrapOptions) -> Self {
        let now_ms = config.start_time_ms.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0)
        });
        let days = config.history_days.clamp(1, 14) as i64;
        let tz_s = opts.tz_offset_min as i64 * 60;
        let anchor = (now_ms / 1000 + tz_s).div_euclid(86_400) - days - 2;
        let phys = Physiology::new(config.seed, config.profile.clone(), opts.tz_offset_min, anchor);
        let mut s = EmulatedStrap {
            family: config.family,
            name: config.device_name.clone(),
            id: mk_id(config.seed, config.family),
            time_scale: if config.time_scale > 0.0 { config.time_scale } else { 1.0 },
            opts,
            phys,
            subs: HashSet::new(),
            connected: true,
            rx: Reassembler::new(config.family),
            outbox: VecDeque::new(),
            now_ms,
            last_tick_ms: now_ms,
            clock_offset_s: 0,
            seq: 0,
            realtime_on: false,
            last_rt_sec: now_ms.div_euclid(1000),
            battery: 62.0 + 30.0 * crate::rng::u01(config.seed, 99, 0),
            charging: false,
            last_batt_pct: 0,
            gaps: Vec::new(),
            wrist_prev: true,
            trim_idx: None,
            sync: None,
            hist_q: VecDeque::new(),
            credit: 0.0,
            history_days: days,
        };
        s.last_batt_pct = s.battery.floor() as i64;
        s.wrist_prev = s.worn(now_ms.div_euclid(1000));
        s.credit = s.opts.chunk_records as f64;
        s
    }

    pub fn family(&self) -> DeviceFamily {
        self.family
    }
    pub fn device_name(&self) -> &str {
        &self.name
    }
    /// Stable per-strap identifier (what the app remembers to reconnect).
    pub fn device_id(&self) -> &str {
        &self.id
    }
    pub fn options(&self) -> &StrapOptions {
        &self.opts
    }
    /// The synthetic-physiology generator (ground truth for tests and tooling).
    pub fn physiology(&self) -> &Physiology {
        &self.phys
    }
    pub fn now_ms(&self) -> i64 {
        self.now_ms
    }
    pub fn battery_percent(&self) -> f64 {
        self.battery
    }
    pub fn is_charging(&self) -> bool {
        self.charging
    }
    pub fn realtime_enabled(&self) -> bool {
        self.realtime_on
    }

    pub fn characteristics(&self) -> Vec<CharacteristicInfo> {
        let mut v: Vec<CharacteristicInfo> = self
            .family
            .characteristics()
            .iter()
            .map(|c| CharacteristicInfo { uuid: c.uuid.to_string(), notify: c.notify, write: c.write })
            .collect();
        v.push(CharacteristicInfo { uuid: STANDARD_HEART_RATE_MEASUREMENT.into(), notify: true, write: false });
        v.push(CharacteristicInfo { uuid: STANDARD_BATTERY_LEVEL.into(), notify: true, write: false });
        v
    }

    /// Host subscribed / unsubscribed to notifications on a characteristic.
    pub fn set_subscribed(&mut self, characteristic: &str, on: bool) {
        let u = characteristic.to_ascii_lowercase();
        if on {
            self.subs.insert(u);
        } else {
            self.subs.remove(&u);
        }
    }

    /// Host link came up (clears stale connection state).
    pub fn on_connect(&mut self, now_ms: i64) {
        self.reset_link();
        self.advance(now_ms);
        self.connected = true;
    }

    /// Host link dropped: subscriptions, realtime streaming and any in-flight history transfer end.
    /// Unacknowledged history stays on the strap and replays on the next transfer.
    pub fn on_disconnect(&mut self) {
        self.reset_link();
        self.connected = false;
    }

    fn reset_link(&mut self) {
        self.subs.clear();
        self.rx.reset();
        self.outbox.clear();
        self.realtime_on = false;
        self.sync = None;
        self.hist_q.clear();
    }

    // ---- dev controls ----

    pub fn take_off(&mut self, now_ms: i64) {
        self.advance(now_ms);
        let s = now_ms.div_euclid(1000);
        if !self.gaps.iter().any(|g| g.1 == i64::MAX) {
            self.gaps.push((s, i64::MAX));
        }
        self.sync_wrist(now_ms);
    }

    pub fn put_on(&mut self, now_ms: i64) {
        self.advance(now_ms);
        let s = now_ms.div_euclid(1000);
        for g in self.gaps.iter_mut().rev() {
            if g.1 == i64::MAX {
                g.1 = s;
                break;
            }
        }
        self.sync_wrist(now_ms);
    }

    pub fn set_battery(&mut self, pct: f64, now_ms: i64) {
        self.advance(now_ms);
        self.battery = pct.clamp(0.0, 100.0);
        self.battery_notify(true);
    }

    pub fn set_charging(&mut self, on: bool, now_ms: i64) {
        self.advance(now_ms);
        if on == self.charging {
            return;
        }
        let s = now_ms.div_euclid(1000);
        self.charging = on;
        if on {
            self.gaps.push((s, i64::MAX));
        } else if let Some(g) = self.gaps.iter_mut().rev().find(|g| g.1 == i64::MAX) {
            g.1 = s;
        }
        let ev = if on { event_number::CHARGING_ON } else { event_number::CHARGING_OFF };
        self.push_event(Event::simple(0, ev, self.strap_unix()));
        self.battery_notify(true);
        self.sync_wrist(now_ms);
    }

    fn sync_wrist(&mut self, now_ms: i64) {
        let w = self.worn(now_ms.div_euclid(1000));
        if w != self.wrist_prev {
            self.wrist_prev = w;
            let n = if w { event_number::WRIST_ON } else { event_number::WRIST_OFF };
            self.push_event(Event::simple(0, n, self.strap_unix()));
        }
    }

    // ---- state helpers ----

    fn in_gap(&self, t: i64) -> bool {
        self.gaps.iter().any(|g| t >= g.0 && t < g.1)
    }

    /// On the wrist (physiology plan and manual controls) at second `t`.
    pub fn worn(&self, t: i64) -> bool {
        self.phys.on_wrist(t) && !self.in_gap(t)
    }

    fn strap_unix(&self) -> u32 {
        (self.now_ms.div_euclid(1000) + self.clock_offset_s).clamp(0, u32::MAX as i64) as u32
    }

    fn next_seq(&mut self) -> u8 {
        let s = self.seq;
        self.seq = self.seq.wrapping_add(1);
        s
    }

    fn uuid(&self, role: CharacteristicRole) -> String {
        self.family.characteristic(role).map(|c| c.uuid.to_string()).unwrap_or_default()
    }

    fn frags(&self, uuid: &str, bytes: &[u8]) -> Vec<Notification> {
        fragment(bytes, self.opts.att_mtu)
            .into_iter()
            .map(|b| Notification { characteristic: uuid.to_string(), bytes: b })
            .collect()
    }

    fn push_packet(&mut self, role: CharacteristicRole, p: Packet) {
        let bytes = p.encode(self.family).expect("emulator packets are encodable");
        let u = self.uuid(role);
        let n = self.frags(&u, &bytes);
        self.outbox.extend(n);
    }

    fn push_event(&mut self, mut ev: Event) {
        if !self.connected {
            return;
        }
        ev.seq = self.next_seq();
        self.push_packet(CharacteristicRole::Events, Packet::Event(ev));
    }

    fn respond(&mut self, cmd: u8, origin: u8, result: CommandResult, body: Vec<u8>) {
        let seq = self.next_seq();
        let r = CommandResponse { seq, cmd, origin_seq: origin, result, body };
        self.push_packet(CharacteristicRole::CommandResponse, Packet::CommandResponse(r));
    }

    fn battery_mv(&self) -> u16 {
        (3300.0 + 900.0 * self.battery / 100.0) as u16
    }

    fn battery_notify(&mut self, force: bool) {
        let pct = self.battery.floor() as i64;
        if !force && pct == self.last_batt_pct {
            return;
        }
        self.last_batt_pct = pct;
        if !self.connected {
            return;
        }
        let b = BatteryEvent { percent: self.battery as f32, millivolts: self.battery_mv(), charging: self.charging };
        let unix = self.strap_unix();
        self.push_event(Event::battery(0, unix, b));
        let u = STANDARD_BATTERY_LEVEL.to_string();
        let n = self.frags(&u, &[self.battery.round().clamp(0.0, 100.0) as u8]);
        self.outbox.extend(n);
    }

    /// Advance the clock; emits wrist / battery events.
    fn advance(&mut self, now_ms: i64) {
        if now_ms <= self.now_ms {
            return;
        }
        let dt = (now_ms - self.now_ms) as f64 / 1000.0;
        self.now_ms = now_ms;
        if self.charging {
            self.battery = (self.battery + CHARGE_PCT_PER_S * dt).min(100.0);
        } else {
            self.battery = (self.battery - DRAIN_PCT_PER_S * dt).max(0.0);
        }
        self.battery_notify(false);
        self.sync_wrist(now_ms);
    }

    // ---- history storage ----

    fn interval(&self) -> i64 {
        self.opts.record_interval_s.max(1) as i64
    }

    /// Newest record index that is fully elapsed.
    fn latest_idx(&self) -> i64 {
        (self.now_ms.div_euclid(1000) - 1).div_euclid(self.interval())
    }

    /// Oldest record index still on the strap (older data has been overwritten or acked/trimmed).
    fn oldest_idx(&self) -> i64 {
        let iv = self.interval();
        let floor = (self.now_ms.div_euclid(1000) - self.history_days * 86_400 + iv - 1).div_euclid(iv);
        match self.trim_idx {
            Some(t) => floor.max(t + 1),
            None => floor,
        }
    }

    /// `(oldest, newest)` record timestamps (Unix s) currently stored, ignoring wrist gaps.
    pub fn history_bounds(&self) -> (i64, i64) {
        (self.oldest_idx() * self.interval(), self.latest_idx() * self.interval())
    }

    /// Last acknowledged record index, if any.
    pub fn trim_index(&self) -> Option<i64> {
        self.trim_idx
    }

    fn record_frame(&self, idx: i64) -> Option<Vec<u8>> {
        let t = idx * self.interval();
        if self.in_gap(t) {
            return None;
        }
        let s = self.phys.sample(t)?;
        let w5 = self.family == DeviceFamily::Whoop5;
        let act = match s.activity {
            Activity::Sleep(_) | Activity::Sit => 0,
            Activity::Walk => 1,
            Activity::Run => 2,
        };
        let sample = HistorySample {
            unix_ms: t * 1000,
            bpm: Some(s.bpm),
            rr_ms: s.rr_ms,
            skin_temp_c: Some(s.temp_c),
            skin_temp_raw: None,
            motion: Some(Motion {
                gravity: Some(s.gravity),
                dynamic_accel_g: w5.then_some(s.motion_g),
                activity: w5.then_some(act),
            }),
            spo2: (!w5).then_some(Spo2Raw { red: s.spo2_red, ir: s.spo2_ir }),
            steps: w5.then_some(s.steps),
        };
        let rec = HistoryRecord::new(self.family, idx as u32, sample);
        let frame = rec.to_frame(self.family, &CodecOptions::default()).ok()?;
        frame.encode(self.family).ok()
    }

    fn token(&self, end_idx: i64) -> Vec<u8> {
        let mut t = (end_idx as u32).to_le_bytes().to_vec();
        t.extend_from_slice(&16u32.to_le_bytes());
        t
    }

    fn data_range(&self) -> DataRange {
        let first = self.oldest_idx();
        let last = self.latest_idx().max(first);
        let read = self.sync.as_ref().map(|s| s.next_idx).unwrap_or(first);
        let page = |i: i64| ((i.max(0) / RECORDS_PER_PAGE) % CAPACITY_PAGES as i64) as u32;
        let ts = |i: i64| ((i * self.interval()).clamp(0, u32::MAX as i64)) as u32;
        let pages = ((last - first).max(0) / RECORDS_PER_PAGE) as u32;
        DataRange {
            boundary_a: page(first),
            read_page: page(read),
            write_page: page(last),
            trim_page: page(first),
            wrap: ((last.max(0) / RECORDS_PER_PAGE) / CAPACITY_PAGES as i64) as u32,
            capacity_pages: CAPACITY_PAGES,
            record_estimate: pages * RECORDS_PER_PAGE as u32,
            derived_backlog: pages,
            clocks: [(ts(first), 0), (ts(first), 0), (ts(read), 0), (ts(last), 0)],
        }
    }

    fn history_meta(&mut self, m: HistoryMetadata) -> Vec<Notification> {
        let p = Packet::HistoryMetadata(m);
        let bytes = p.encode(self.family).expect("metadata encodable");
        let u = self.uuid(CharacteristicRole::Data);
        self.frags(&u, &bytes)
    }

    fn start_sync(&mut self) {
        self.hist_q.clear();
        self.sync = Some(SyncRun { next_idx: self.oldest_idx(), phase: Phase::SendStart });
    }

    fn build_chunk(&mut self) {
        let Some(run) = &self.sync else { return };
        let mut idx = run.next_idx;
        let latest = self.latest_idx();
        let mut frames: Vec<Vec<u8>> = Vec::new();
        let mut last = idx - 1;
        while idx <= latest && frames.len() < self.opts.chunk_records {
            if let Some(f) = self.record_frame(idx) {
                frames.push(f);
                last = idx;
            }
            idx += 1;
        }
        if frames.is_empty() {
            let seq = self.next_seq();
            let n = self.history_meta(HistoryMetadata::Complete { seq, payload: vec![0; 4] });
            self.hist_q.push_back(HistItem { notifs: n, cost: 0.0 });
            self.sync.as_mut().unwrap().phase = Phase::Done;
            return;
        }
        let u = self.uuid(CharacteristicRole::Data);
        for f in &frames {
            let n = self.frags(&u, f);
            self.hist_q.push_back(HistItem { notifs: n, cost: 1.0 });
        }
        let seq = self.next_seq();
        let end = HistoryEnd {
            unix: (last * self.interval()) as u32,
            subsec: 0,
            reserved: 0,
            ack_token: self.token(last),
        };
        let n = self.history_meta(HistoryMetadata::end(seq, end));
        self.hist_q.push_back(HistItem { notifs: n, cost: 0.0 });
        let run = self.sync.as_mut().unwrap();
        run.next_idx = idx.max(last + 1);
        run.phase = Phase::Waiting { end_idx: last };
    }

    // ---- host writes ----

    /// Host wrote bytes (a command frame, possibly fragmented) to a characteristic.
    pub fn on_write(&mut self, characteristic: &str, bytes: &[u8], now_ms: i64) {
        self.advance(now_ms);
        if !characteristic.eq_ignore_ascii_case(self.family.command_characteristic_uuid()) {
            return;
        }
        let frames: Vec<Frame> = self
            .rx
            .push(bytes)
            .into_iter()
            .filter_map(|r| match r {
                Reassembled::Frame { frame, .. } => Some(frame),
                Reassembled::Corrupt { .. } => None,
            })
            .collect();
        for f in frames {
            if let Ok(cmd) = HostCommand::from_frame(&f, self.family) {
                self.handle(cmd, f.seq, f.cmd);
            }
        }
    }

    fn handle(&mut self, cmd: HostCommand, origin: u8, num: u8) {
        let fam = self.family;
        let ok = CommandResult::Success;
        match cmd {
            HostCommand::ToggleRealtimeHr(on) => {
                self.realtime_on = on;
                self.last_rt_sec = self.now_ms.div_euclid(1000) - self.realtime_period_s();
                self.respond(num, origin, ok, vec![]);
                let n = if on { event_number::BLE_REALTIME_HR_ON } else { event_number::BLE_REALTIME_HR_OFF };
                self.push_event(Event::simple(0, n, self.strap_unix()));
            }
            HostCommand::GetBatteryLevel => {
                let body = ResponseBody::Battery { percent: self.battery as f32 }.encode(fam);
                self.respond(num, origin, ok, body);
            }
            HostCommand::GetExtendedBatteryInfo => {
                let body = ResponseBody::ExtendedBattery { millivolts: self.battery_mv() }.encode(fam);
                self.respond(num, origin, ok, body);
            }
            HostCommand::GetClock => {
                let unix = self.strap_unix();
                let ticks = ((self.now_ms.rem_euclid(1000)) * 32768 / 1000) as u16;
                let c = ClockReading { unix, subsec_ticks: (fam == DeviceFamily::Whoop5).then_some(ticks) };
                self.respond(num, origin, ok, ResponseBody::Clock(c).encode(fam));
            }
            HostCommand::SetClock { unix, .. } => {
                self.clock_offset_s = unix as i64 - self.now_ms.div_euclid(1000);
                self.respond(num, origin, ok, vec![]);
            }
            HostCommand::GetDataRange => {
                if fam == DeviceFamily::Whoop5 {
                    self.respond(num, origin, CommandResult::Pending, vec![]);
                }
                let body = ResponseBody::DataRange(self.data_range()).encode(fam);
                self.respond(num, origin, ok, body);
            }
            HostCommand::SendHistoricalData => {
                self.respond(num, origin, ok, vec![0]);
                self.start_sync();
            }
            HostCommand::HistoricalDataResult { ack_token } => {
                if ack_token.len() >= 4 {
                    let idx = u32::from_le_bytes([ack_token[0], ack_token[1], ack_token[2], ack_token[3]]) as i64;
                    let accept = match self.sync.as_ref().map(|s| &s.phase) {
                        Some(Phase::Waiting { end_idx }) => *end_idx == idx,
                        _ => idx >= self.oldest_idx() && idx <= self.latest_idx(),
                    };
                    if accept {
                        self.trim_idx = Some(self.trim_idx.map_or(idx, |t| t.max(idx)));
                        if let Some(run) = self.sync.as_mut()
                            && matches!(run.phase, Phase::Waiting { .. }) {
                                run.phase = Phase::NeedChunk;
                            }
                    }
                }
                self.respond(num, origin, ok, vec![]);
            }
            HostCommand::AbortHistoricalTransmits => {
                self.sync = None;
                self.hist_q.clear();
                self.respond(num, origin, ok, vec![]);
            }
            HostCommand::GetHello => {
                // Opaque identity block (the real one carries serial / key material; this is filler).
                let mut b = vec![if fam == DeviceFamily::Whoop5 { 1 } else { 0 }];
                b.extend_from_slice(self.id.as_bytes());
                b.resize(40, 0);
                self.respond(num, origin, ok, b);
            }
            HostCommand::GetVersionInfo => {
                let v = VersionInfo { harvard: [41, 17, 6, 0], boylston: [17, 2, 2, 0] };
                self.respond(num, origin, ok, ResponseBody::VersionInfo(v).encode(fam));
            }
            HostCommand::Raw { cmd, .. } => {
                let _ = cmd;
                self.respond(num, origin, CommandResult::Unsupported, vec![]);
            }
        }
    }

    // ---- time ----

    /// Emulated seconds between live HR readings, so they arrive about once per wall-clock second.
    fn realtime_period_s(&self) -> i64 {
        (self.time_scale.round() as i64).max(1)
    }

    /// Advance the emulated clock to `now_ms` (records new samples into on-strap history) and
    /// return every notification now due: command responses, live HR stream, history chunks.
    pub fn tick(&mut self, now_ms: i64) -> Vec<Notification> {
        let prev_tick = self.last_tick_ms;
        self.advance(now_ms);
        let now_ms = self.now_ms;
        self.last_tick_ms = now_ms;
        let mut out: Vec<Notification> = Vec::new();

        // live heart rate: one reading per *real* second, like the real strap, even when the
        // emulated clock is fast-forwarded (time_scale 60 ⇒ one reading every 60 emulated s).
        if self.realtime_on {
            let sec = now_ms.div_euclid(1000);
            if sec - self.last_rt_sec >= self.realtime_period_s() {
                self.last_rt_sec = sec;
                if self.worn(sec)
                    && let Some(s) = self.phys.sample(sec) {
                        let rt = RealtimeHr {
                            seq: self.next_seq(),
                            device_seconds: (sec + self.clock_offset_s).clamp(0, u32::MAX as i64) as u32,
                            subsec: (now_ms.rem_euclid(1000) * 32768 / 1000) as u16,
                            bpm: s.bpm,
                            rr_ms: s.rr_ms.iter().copied().take(5).collect(),
                        };
                        self.push_packet(CharacteristicRole::Data, Packet::RealtimeHr(rt));
                        let std = StdHeartRate { bpm: s.bpm as u16, contact: Some(true), energy_kj: None, rr_ms: s.rr_ms.clone() }.encode();
                        let n = self.frags(STANDARD_HEART_RATE_MEASUREMENT, &std);
                        self.outbox.extend(n);
                    }
            }
        }

        // history streaming
        let data_uuid = self.uuid(CharacteristicRole::Data);
        let data_ok = self.subs.contains(&data_uuid);
        if let Some(rate) = self.opts.history_records_per_sec {
            let dt_real = ((now_ms - prev_tick).max(0) as f64 / 1000.0) / self.time_scale;
            let cap = rate.max(self.opts.chunk_records as f64);
            self.credit = (self.credit + rate * dt_real).min(cap);
        }
        let unlimited = self.opts.history_records_per_sec.is_none();
        if data_ok {
            loop {
                if self.hist_q.is_empty() {
                    let Some(run) = &self.sync else { break };
                    match run.phase {
                        Phase::SendStart => {
                            let seq = self.next_seq();
                            let payload = if self.family == DeviceFamily::Whoop5 {
                                let mut p = ((self.oldest_idx() * self.interval()) as u32).to_le_bytes().to_vec();
                                p.resize(36, 0);
                                p
                            } else {
                                vec![]
                            };
                            let n = self.history_meta(HistoryMetadata::Start { seq, payload });
                            self.hist_q.push_back(HistItem { notifs: n, cost: 0.0 });
                            self.sync.as_mut().unwrap().phase = Phase::NeedChunk;
                        }
                        Phase::NeedChunk => self.build_chunk(),
                        Phase::Waiting { .. } => break,
                        Phase::Done => {
                            self.sync = None;
                            break;
                        }
                    }
                    continue;
                }
                let front_cost = self.hist_q.front().map(|h| h.cost).unwrap_or(0.0);
                if !unlimited && front_cost > self.credit {
                    break;
                }
                let item = self.hist_q.pop_front().unwrap();
                self.credit -= item.cost;
                out.extend(item.notifs);
                if unlimited && out.len() > 200_000 {
                    break;
                }
            }
        }
        // Order: responses/events first, then history.
        let mut res: Vec<Notification> = Vec::new();
        while let Some(n) = self.outbox.pop_front() {
            if self.subs.contains(&n.characteristic.to_ascii_lowercase()) {
                res.push(n);
            }
        }
        res.extend(out);
        res
    }
}

