#![allow(dead_code)]
//! Shared helpers for the emulator integration tests.

use unstrapped_core::metrics::{HrSample, MotionSample, RrInterval, Spo2Sample, StepSample, TempSample};
use unstrapped_core::protocol::{
    CodecOptions, DeviceFamily, HistoricalPacket, HistoryRecord, HistorySample, Motion, Spo2Raw, Packet,
};
use unstrapped_emulator::physio::{Activity, Physiology, spo2_from_raw};
use unstrapped_emulator::{EmulatorConfig, SubjectProfile};

pub const T0: i64 = 1_767_225_600_000; // 2026-01-01T00:00:00Z

pub fn cfg(family: DeviceFamily) -> EmulatorConfig {
    EmulatorConfig {
        family,
        seed: 7,
        history_days: 3,
        start_time_ms: Some(T0 + 14 * 3_600_000 + 5 * 60_000), // 14:05 UTC
        time_scale: 1.0,
        profile: SubjectProfile::default(),
        device_name: "WHOOP 4EMU0001".into(),
    }
}

#[derive(Default)]
pub struct Streams {
    pub hr: Vec<HrSample>,
    pub rr: Vec<RrInterval>,
    pub motion: Vec<MotionSample>,
    pub temp: Vec<TempSample>,
    pub spo2: Vec<Spo2Sample>,
    pub steps: Vec<StepSample>,
}

/// Turn one second of physiology into the record a host would decode (through the real codec).
pub fn decoded_sample(phys: &Physiology, family: DeviceFamily, t: i64) -> Option<HistorySample> {
    let s = phys.sample(t)?;
    let w5 = family == DeviceFamily::Whoop5;
    let sample = HistorySample {
        unix_ms: t * 1000,
        bpm: Some(s.bpm),
        rr_ms: s.rr_ms,
        skin_temp_c: Some(s.temp_c),
        skin_temp_raw: None,
        motion: Some(Motion { gravity: Some(s.gravity), dynamic_accel_g: w5.then_some(s.motion_g), activity: None }),
        spo2: (!w5).then_some(Spo2Raw { red: s.spo2_red, ir: s.spo2_ir }),
        steps: w5.then_some(s.steps),
    };
    let f = HistoryRecord::new(family, t as u32, sample).to_frame(family, &CodecOptions::default()).unwrap();
    match Packet::decode(&f, family).unwrap() {
        Packet::Historical(HistoricalPacket::Record(r)) => Some(r.sample),
        _ => None,
    }
}

/// Convert decoded history samples (in time order) to the metrics engine's input streams.
pub fn to_streams(samples: &[HistorySample], family: DeviceFamily) -> Streams {
    let mut st = Streams::default();
    let mut prev_g: Option<([f32; 3], i64)> = None;
    let mut prev_steps: Option<(u32, i64)> = None;
    for s in samples {
        let ts = s.unix_ms;
        if let Some(b) = s.bpm {
            st.hr.push(HrSample { ts_ms: ts, bpm: b as f64 });
        }
        for r in &s.rr_ms {
            st.rr.push(RrInterval { ts_ms: ts, rr_ms: *r as f64 });
        }
        if let Some(c) = s.skin_temp_c {
            st.temp.push(TempSample { ts_ms: ts, celsius: c as f64 });
        }
        if let Some(sp) = s.spo2 {
            st.spo2.push(Spo2Sample { ts_ms: ts, percent: spo2_from_raw(sp.red, sp.ir) });
        }
        if let Some(g) = s.motion.and_then(|m| m.gravity) {
            if let Some((pg, pt)) = prev_g
                && ts - pt <= 2000 {
                    let d = ((g[0] - pg[0]).powi(2) + (g[1] - pg[1]).powi(2) + (g[2] - pg[2]).powi(2)).sqrt();
                    st.motion.push(MotionSample { ts_ms: ts, magnitude_g: d as f64 });
                }
            prev_g = Some((g, ts));
        }
        if family == DeviceFamily::Whoop5
            && let Some(c) = s.steps {
                if let Some((pc, pt)) = prev_steps
                    && ts - pt <= 2000 {
                        let add = (c + 65536 - pc) % 65536;
                        st.steps.push(StepSample { ts_ms: ts, steps: add });
                    }
                prev_steps = Some((c, ts));
            }
    }
    st
}

/// Generate decoded samples for `[from_s, to_s)` straight from the physiology.
pub fn gen_range(phys: &Physiology, family: DeviceFamily, from_s: i64, to_s: i64) -> Vec<HistorySample> {
    (from_s..to_s).filter_map(|t| decoded_sample(phys, family, t)).collect()
}

pub fn rmssd(rr: &[f64]) -> f64 {
    let d: Vec<f64> = rr.windows(2).map(|w| (w[1] - w[0]).powi(2)).collect();
    (d.iter().sum::<f64>() / d.len() as f64).sqrt()
}

pub fn is_sleep(a: Activity) -> bool {
    matches!(a, Activity::Sleep(_))
}

// ---------------------------------------------------------------------------------------------
// A tiny host: what the app does with unstrapped-core, driven against an EmulatedStrap directly.
// ---------------------------------------------------------------------------------------------

use std::collections::{BTreeMap, VecDeque};
use unstrapped_core::protocol::gatt::{STANDARD_BATTERY_LEVEL, STANDARD_HEART_RATE_MEASUREMENT};
use unstrapped_core::protocol::{
    CommandResponse, Event, HistorySync, HostCommand, PendingChunk, Reassembled, ReassemblerSet, RealtimeHr, SeqCounter,
    StdHeartRate, SyncEvent, fragment,
};
use unstrapped_emulator::{EmulatedStrap, StrapOptions};

pub fn fast_opts(interval_s: u32) -> StrapOptions {
    StrapOptions { record_interval_s: interval_s, history_records_per_sec: None, tz_offset_min: 0, ..StrapOptions::default() }
}

pub struct Host {
    pub strap: EmulatedStrap,
    pub fam: DeviceFamily,
    pub now: i64,
    rs: ReassemblerSet,
    seq: SeqCounter,
    pub sync: HistorySync,
    pub std_hr: Vec<StdHeartRate>,
    pub std_battery: Vec<u8>,
    pub responses: Vec<CommandResponse>,
    pub events: Vec<Event>,
    pub realtime: Vec<RealtimeHr>,
    pub chunks: VecDeque<PendingChunk>,
    pub started: bool,
    pub completed: bool,
}

impl Host {
    pub fn new(cfg: EmulatorConfig, opts: StrapOptions) -> Host {
        let fam = cfg.family;
        let strap = EmulatedStrap::with_options(cfg, opts);
        let now = strap.now_ms();
        let mut h = Host {
            strap,
            fam,
            now,
            rs: ReassemblerSet::new(fam),
            seq: SeqCounter::new(1),
            sync: HistorySync::new(),
            std_hr: vec![],
            std_battery: vec![],
            responses: vec![],
            events: vec![],
            realtime: vec![],
            chunks: VecDeque::new(),
            started: false,
            completed: false,
        };
        h.subscribe_all();
        h
    }

    pub fn subscribe_all(&mut self) {
        for c in self.strap.characteristics() {
            if c.notify {
                self.strap.set_subscribed(&c.uuid, true);
            }
        }
    }

    /// Reconnect: link drops, comes back, host resubscribes.
    pub fn reconnect(&mut self) {
        self.strap.on_disconnect();
        self.rs.reset();
        self.sync.reset();
        self.chunks.clear();
        self.completed = false;
        self.started = false;
        self.strap.on_connect(self.now);
        self.subscribe_all();
    }

    pub fn send(&mut self, cmd: HostCommand) -> u8 {
        let seq = self.seq.next_seq();
        let bytes = cmd.encode(self.fam, seq).unwrap();
        let uuid = self.fam.command_characteristic_uuid();
        for frag in fragment(&bytes, 23) {
            self.strap.on_write(uuid, &frag, self.now);
        }
        seq
    }

    pub fn pump(&mut self) {
        let notifs = self.strap.tick(self.now);
        for n in notifs {
            if n.characteristic == STANDARD_HEART_RATE_MEASUREMENT {
                self.std_hr.push(StdHeartRate::decode(&n.bytes).unwrap());
                continue;
            }
            if n.characteristic == STANDARD_BATTERY_LEVEL {
                self.std_battery.push(n.bytes[0]);
                continue;
            }
            let items = self.rs.push_uuid(&n.characteristic, &n.bytes).expect("known characteristic");
            for it in items {
                let Reassembled::Frame { frame, raw } = it else { panic!("corrupt frame from strap") };
                let pkt = Packet::decode(&frame, self.fam).expect("decodes");
                match &pkt {
                    Packet::CommandResponse(r) => self.responses.push(r.clone()),
                    Packet::RealtimeHr(r) => self.realtime.push(r.clone()),
                    Packet::Event(e) => self.events.push(e.clone()),
                    _ => {}
                }
                match self.sync.on_packet(&pkt, &raw) {
                    Some(SyncEvent::Started) => self.started = true,
                    Some(SyncEvent::Chunk(c)) => self.chunks.push_back(c),
                    Some(SyncEvent::Completed) => self.completed = true,
                    None => {}
                }
            }
        }
    }

    pub fn advance_ms(&mut self, ms: i64) {
        self.now += ms;
        self.pump();
    }

    /// Run the real history sync loop: persist each chunk, then ack it. Stops after `max_acks`
    /// chunks have been acknowledged (the next chunk is persisted but left unacked) or on Complete.
    /// Returns (chunks seen, unacked chunk if any).
    pub fn sync_history(&mut self, store: &mut BTreeMap<i64, HistoryRecord>, max_acks: usize) -> (usize, Option<PendingChunk>) {
        self.completed = false;
        self.send(HostCommand::SendHistoricalData);
        let (mut seen, mut acked) = (0, 0);
        for _ in 0..2_000_000 {
            self.pump();
            while let Some(c) = self.chunks.pop_front() {
                seen += 1;
                for r in &c.records {
                    store.insert(r.sample.unix_ms, r.clone());
                }
                if acked >= max_acks {
                    return (seen, Some(c));
                }
                acked += 1;
                let ack = c.ack();
                self.send(ack);
            }
            if self.completed {
                return (seen, None);
            }
        }
        panic!("history sync did not complete");
    }

    pub fn last_response(&self, cmd: u8) -> Option<&CommandResponse> {
        self.responses.iter().rev().find(|r| r.cmd == cmd)
    }
}
