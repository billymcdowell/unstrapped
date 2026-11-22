mod common;
use std::collections::BTreeMap;

use common::*;
use unstrapped_core::protocol::{
    CommandResult, DataRange, DeviceFamily, HistoryRecord, HostCommand, Packet, ResponseBody, command_number, event_number,
};

const FAMS: [DeviceFamily; 2] = [DeviceFamily::Whoop4, DeviceFamily::Whoop5];

fn cfg_days(f: DeviceFamily, days: u32) -> unstrapped_emulator::EmulatorConfig {
    unstrapped_emulator::EmulatorConfig { history_days: days, ..cfg(f) }
}

#[test]
fn connect_battery_clock_and_realtime() {
    for fam in FAMS {
        let mut h = Host::new(cfg(fam), fast_opts(5));
        let chars = h.strap.characteristics();
        for c in fam.characteristics() {
            assert!(chars.iter().any(|x| x.uuid == c.uuid && x.notify == c.notify && x.write == c.write), "{fam:?} missing {}", c.uuid);
        }
        if fam == DeviceFamily::Whoop5 {
            h.send(HostCommand::GetHello);
            h.pump();
            assert!(h.last_response(145).is_some());
        } else {
            h.send(HostCommand::GetVersionInfo);
            h.pump();
            let r = h.last_response(command_number::REPORT_VERSION_INFO).unwrap();
            assert!(matches!(r.typed_body(fam).unwrap(), ResponseBody::VersionInfo(_)));
        }
        h.send(HostCommand::GetBatteryLevel);
        h.pump();
        let r = h.last_response(command_number::GET_BATTERY_LEVEL).unwrap();
        assert_eq!(r.result, CommandResult::Success);
        let ResponseBody::Battery { percent } = r.typed_body(fam).unwrap() else { panic!() };
        assert!((50.0..=100.0).contains(&percent), "{percent}");

        // clock round trip
        h.send(HostCommand::GetClock);
        h.pump();
        let clk_cmd = if fam == DeviceFamily::Whoop5 { command_number::GET_CLOCK_W5 } else { command_number::GET_CLOCK };
        let ResponseBody::Clock(c) = h.last_response(clk_cmd).unwrap().typed_body(fam).unwrap() else { panic!() };
        assert_eq!(c.unix as i64, h.now / 1000);
        h.send(HostCommand::SetClock { unix: (h.now / 1000 + 90) as u32, subsec_ticks: 0 });
        h.pump();
        h.responses.clear();
        h.send(HostCommand::GetClock);
        h.pump();
        let ResponseBody::Clock(c) = h.last_response(clk_cmd).unwrap().typed_body(fam).unwrap() else { panic!() };
        assert_eq!(c.unix as i64, h.now / 1000 + 90);

        // realtime HR
        h.send(HostCommand::ToggleRealtimeHr(true));
        for _ in 0..12 {
            h.advance_ms(1000);
        }
        assert!(h.realtime.len() >= 10, "{fam:?} got {}", h.realtime.len());
        assert!(h.std_hr.len() >= 10);
        assert!(h.realtime.iter().all(|r| (35..=200).contains(&r.bpm)), "bpm range");
        assert!(h.realtime.iter().flat_map(|r| r.rr_ms.iter()).all(|r| (300..=2000).contains(r)));
        assert!(h.realtime.iter().any(|r| !r.rr_ms.is_empty()));
        assert!(h.std_hr.iter().all(|s| s.contact == Some(true) && (35..=200).contains(&s.bpm)));
        assert!(h.events.iter().any(|e| e.number == event_number::BLE_REALTIME_HR_ON));
        // off again: stream stops
        h.send(HostCommand::ToggleRealtimeHr(false));
        h.pump();
        let n = h.realtime.len();
        for _ in 0..3 {
            h.advance_ms(1000);
        }
        assert_eq!(h.realtime.len(), n);
    }
}

#[test]
fn realtime_hr_is_paced_by_wall_clock_when_time_is_fast_forwarded() {
    // time_scale 60: one emulated minute passes per real second. Live HR must still arrive about
    // once per real second (every 60 emulated s), not 60 readings per real second.
    let mut h = Host::new(unstrapped_emulator::EmulatorConfig { time_scale: 60.0, ..cfg(DeviceFamily::Whoop4) }, fast_opts(5));
    h.send(HostCommand::ToggleRealtimeHr(true));
    for _ in 0..10 * 60 {
        h.advance_ms(1000); // 10 real seconds = 600 emulated seconds
    }
    assert!((10..=11).contains(&h.realtime.len()), "got {} readings in 10 real seconds", h.realtime.len());
    assert!((10..=11).contains(&h.std_hr.len()), "got {} std HR readings", h.std_hr.len());
}

#[test]
fn only_subscribed_characteristics_notify() {
    let mut h = Host::new(cfg(DeviceFamily::Whoop4), fast_opts(5));
    let data = DeviceFamily::Whoop4.characteristics()[3].uuid;
    h.strap.set_subscribed(data, false);
    h.strap.set_subscribed(unstrapped_core::protocol::gatt::STANDARD_HEART_RATE_MEASUREMENT, false);
    h.send(HostCommand::ToggleRealtimeHr(true));
    for _ in 0..5 {
        h.advance_ms(1000);
    }
    assert!(h.realtime.is_empty() && h.std_hr.is_empty());
    assert!(h.last_response(command_number::TOGGLE_REALTIME_HR).is_some(), "responses still arrive");
}

fn data_range(h: &mut Host) -> DataRange {
    h.responses.clear();
    h.send(HostCommand::GetDataRange);
    h.pump();
    let r = h.responses.iter().rev().find(|r| r.result == CommandResult::Success && r.cmd == command_number::GET_DATA_RANGE).unwrap();
    let ResponseBody::DataRange(d) = r.typed_body(h.fam).unwrap() else { panic!() };
    d
}

#[test]
fn full_history_sync_covers_window_without_gaps() {
    for fam in FAMS {
        let iv = 5i64;
        let days = 2;
        let mut h = Host::new(cfg_days(fam, days), fast_opts(iv as u32));
        let now_s = h.now / 1000;
        let range = data_range(&mut h);
        let oldest = range.oldest_unix().unwrap() as i64;
        assert!((oldest - (now_s - days as i64 * 86_400)).abs() <= iv, "range oldest {oldest}");
        if fam == DeviceFamily::Whoop5 {
            assert!(h.responses.iter().any(|r| r.result == CommandResult::Pending), "W5 pending before data range");
        }

        let mut store: BTreeMap<i64, HistoryRecord> = BTreeMap::new();
        let (chunks, pending) = h.sync_history(&mut store, usize::MAX);
        assert!(pending.is_none());
        assert!(h.started && h.completed);
        assert!(chunks > 100);
        let first = *store.keys().next().unwrap() / 1000;
        let last = *store.keys().next_back().unwrap() / 1000;
        assert!(first - (now_s - days as i64 * 86_400) <= iv, "first {first}");
        assert!(now_s - last <= iv + 1, "last {last} now {now_s}");

        // No gaps except planned off-wrist stretches.
        let phys = h.strap.physiology();
        let ts: Vec<i64> = store.keys().map(|t| t / 1000).collect();
        let mut gaps = 0;
        for w in ts.windows(2) {
            if w[1] - w[0] > iv {
                gaps += 1;
                let d = phys.local_day(w[0] as f64);
                let plan = phys.day_plan(d);
                let (a, b) = plan.off_wrist.expect("gap without a planned off-wrist period");
                assert!(w[0] as f64 >= a - iv as f64 - 1.0 && (w[1] as f64) <= b + iv as f64 + 1.0, "unexplained gap {w:?}");
            }
            assert!(w[1] > w[0]);
        }
        println!("{fam:?}: {} records, {} deliberate gaps", ts.len(), gaps);
        assert!(gaps >= 1, "expected at least one off-wrist gap in {days} days");
        // Decoded content is sane.
        for r in store.values().step_by(37) {
            let b = r.sample.bpm.unwrap();
            assert!((35..=200).contains(&b));
            assert!(r.sample.skin_temp_c.unwrap() > 28.0);
            if fam == DeviceFamily::Whoop5 {
                assert!(r.sample.steps.is_some());
            } else {
                assert!(r.sample.spo2.is_some());
            }
        }
    }
}

#[test]
fn unacked_chunk_replays_after_reconnect_and_acked_is_trimmed() {
    for fam in FAMS {
        let mut h = Host::new(cfg_days(fam, 1), fast_opts(5));
        let mut store = BTreeMap::new();
        let (seen, pending) = h.sync_history(&mut store, 10);
        assert_eq!(seen, 11);
        let unacked = pending.expect("11th chunk left unacked");
        let first_unacked = unacked.records.first().unwrap().sample.unix_ms;
        let last_acked = unacked.records.first().unwrap().index; // sanity: records carry an index
        let _ = last_acked;
        // the acked 10 chunks are before the unacked one
        assert!(store.range(..first_unacked).count() >= 10 * 50 - 50);

        // Reconnect and sync again: the strap replays from the first unacked record.
        h.reconnect();
        let mut store2 = BTreeMap::new();
        h.send(HostCommand::SendHistoricalData);
        let mut first_after = None;
        for _ in 0..1000 {
            h.pump();
            if let Some(c) = h.chunks.pop_front() {
                first_after = c.records.first().map(|r| r.sample.unix_ms);
                break;
            }
        }
        assert_eq!(first_after, Some(first_unacked), "{fam:?} replay should start at the unacked chunk");
        let _ = &mut store2;

        // Finish the sync (ack everything) -> everything delivered, and the strap trims.
        h.reconnect();
        let (_, p) = h.sync_history(&mut store2, usize::MAX);
        assert!(p.is_none());
        let newest = *store2.keys().next_back().unwrap();
        let range = data_range(&mut h);
        assert!(range.oldest_unix().unwrap() as i64 * 1000 > newest - 10_000, "trimmed through newest");
        assert!(h.strap.trim_index().is_some());

        // Syncing again returns (almost) nothing: acked data is gone.
        h.reconnect();
        let mut again = BTreeMap::new();
        let (chunks, _) = h.sync_history(&mut again, usize::MAX);
        assert_eq!(chunks, 0);
        assert!(again.is_empty());
        // 20 minutes later: about 20 min of new records.
        h.now += 20 * 60_000;
        h.reconnect();
        let (_, _) = h.sync_history(&mut again, usize::MAX);
        let n = again.len();
        assert!((200..=250).contains(&n), "{fam:?}: new records after 20 min = {n}");
        assert!(again.keys().all(|t| *t > newest));
    }
}

#[test]
fn abort_stops_streaming_without_trimming() {
    let mut h = Host::new(cfg_days(DeviceFamily::Whoop4, 1), fast_opts(5));
    h.send(HostCommand::SendHistoricalData);
    h.pump();
    assert!(!h.chunks.is_empty());
    h.chunks.clear();
    h.send(HostCommand::AbortHistoricalTransmits);
    h.pump();
    h.chunks.clear();
    h.pump();
    h.pump();
    assert!(h.chunks.is_empty(), "no more data after abort");
    assert!(h.strap.trim_index().is_none());
}

#[test]
fn history_is_overwritten_after_history_days() {
    for fam in FAMS {
        let mut h = Host::new(cfg_days(fam, 14), fast_opts(600));
        let start_s = h.now / 1000;
        // 16 days pass with the host away (nothing acked).
        h.now += 16 * 86_400_000;
        h.pump();
        let now_s = h.now / 1000;
        let r = data_range(&mut h);
        let oldest = r.oldest_unix().unwrap() as i64;
        assert!((oldest - (now_s - 14 * 86_400)).abs() <= 600, "{fam:?} oldest {oldest}");
        assert!(oldest > start_s + 86_400, "data from the first two days is gone");
        let mut store = BTreeMap::new();
        let (_, p) = h.sync_history(&mut store, usize::MAX);
        assert!(p.is_none());
        let first = *store.keys().next().unwrap() / 1000;
        assert!(first >= now_s - 14 * 86_400, "{fam:?}: overwritten data must not come back (first {first})");
        let expect = 14 * 144;
        assert!((expect - 40..=expect + 2).contains(&(store.len() as i64)), "{} records", store.len());
    }
}

#[test]
fn wrist_battery_and_charging_events() {
    for fam in FAMS {
        let mut h = Host::new(cfg(fam), fast_opts(5));
        h.pump();
        h.events.clear();
        h.strap.take_off(h.now);
        h.advance_ms(1000);
        assert!(h.events.iter().any(|e| e.number == event_number::WRIST_OFF));
        h.advance_ms(60_000);
        h.strap.put_on(h.now);
        h.advance_ms(1000);
        assert!(h.events.iter().any(|e| e.number == event_number::WRIST_ON));

        h.strap.set_battery(41.0, h.now);
        h.advance_ms(1000);
        let b = h.events.iter().rev().find_map(|e| e.as_battery()).expect("battery event");
        assert!((b.percent - 41.0).abs() < 1.0 && !b.charging);
        assert!(h.std_battery.last() == Some(&41));

        h.strap.set_charging(true, h.now);
        h.advance_ms(1000);
        assert!(h.events.iter().any(|e| e.number == event_number::CHARGING_ON));
        h.advance_ms(20 * 60_000);
        assert!(h.strap.battery_percent() > 41.0 + 10.0, "charges");
        h.strap.set_charging(false, h.now);
        let level = h.strap.battery_percent();
        h.advance_ms(24 * 3_600_000);
        let drop = level - h.strap.battery_percent();
        assert!((10.0..35.0).contains(&drop), "drains ~22%/day, got {drop}");
        // the take-off / charge periods are gaps in recorded history
        let mut store = BTreeMap::new();
        h.sync_history(&mut store, usize::MAX);
        let ts: Vec<i64> = store.keys().map(|t| t / 1000).collect();
        assert!(ts.windows(2).any(|w| w[1] - w[0] > 300), "manual off-wrist leaves a gap");
        let _ = Packet::Unknown;
    }
}
