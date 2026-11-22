mod common;
use std::time::Duration;

use common::*;
use tokio::net::TcpListener;
use tokio::sync::mpsc::Receiver;
use tokio::time::timeout;
use unstrapped_core::protocol::{
    DeviceFamily, HistorySync, HostCommand, Packet, Reassembled, ReassemblerSet, SyncEvent, event_number,
};
use unstrapped_emulator::client::EmulatorClient;
use unstrapped_emulator::server::{ServerOptions, serve_on};
use unstrapped_emulator::wire::EmuMsg;

async fn spawn(family: DeviceFamily, time_scale: f64, fast: bool, interval: u32, days: u32) -> String {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap().to_string();
    let mut c = cfg(family);
    c.time_scale = time_scale;
    c.history_days = days;
    let mut opts = ServerOptions { fast_history: fast, tick_ms: 10, ..Default::default() };
    opts.strap.record_interval_s = interval;
    opts.strap.tz_offset_min = 0;
    tokio::spawn(async move {
        let _ = serve_on(l, c, opts).await;
    });
    addr
}

async fn setup(addr: &str, family: DeviceFamily) -> (EmulatorClient, Receiver<EmuMsg>) {
    let (mut c, adv) = EmulatorClient::connect(addr).await.unwrap();
    match adv {
        EmuMsg::Advertisement { family: f, in_range, service_uuid, device_id, .. } => {
            assert_eq!(f, family);
            assert!(in_range);
            assert_eq!(service_uuid, family.service_uuid());
            assert!(!device_id.is_empty());
        }
        other => panic!("{other:?}"),
    }
    let chars = c.gatt_connect().await.unwrap();
    for ch in chars.iter().filter(|x| x.notify) {
        c.subscribe(&ch.uuid).await.unwrap();
    }
    let ev = c.take_events();
    (c, ev)
}

async fn next_packets(rx: &mut Receiver<EmuMsg>, rs: &mut ReassemblerSet, family: DeviceFamily, wait: Duration) -> Vec<(Packet, Vec<u8>)> {
    let mut out = vec![];
    if let Ok(Some(msg)) = timeout(wait, rx.recv()).await
        && let EmuMsg::Notify { characteristic, hex } = msg {
            let bytes = hex::decode(hex).unwrap();
            if let Some(items) = rs.push_uuid(&characteristic, &bytes) {
                for it in items {
                    if let Reassembled::Frame { frame, raw } = it {
                        out.push((Packet::decode(&frame, family).unwrap(), raw));
                    }
                }
            }
        }
    out
}

#[tokio::test]
async fn live_hr_over_tcp_both_families() {
    for fam in [DeviceFamily::Whoop4, DeviceFamily::Whoop5] {
        let addr = spawn(fam, 20.0, true, 5, 1).await;
        let (mut c, mut ev) = setup(&addr, fam).await;
        c.write(fam.command_characteristic_uuid(), &HostCommand::ToggleRealtimeHr(true).encode(fam, 1).unwrap()).await.unwrap();
        let mut rs = ReassemblerSet::new(fam);
        let mut hr = vec![];
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while hr.len() < 5 && tokio::time::Instant::now() < deadline {
            for (p, _) in next_packets(&mut ev, &mut rs, fam, Duration::from_millis(500)).await {
                if let Packet::RealtimeHr(r) = p {
                    hr.push(r.bpm);
                }
            }
        }
        assert!(hr.len() >= 5, "{fam:?}: only {} live HR packets", hr.len());
        assert!(hr.iter().all(|b| (35..=200).contains(b)));

        // controls: take_off -> wrist-off event
        c.control("take_off", None).await.unwrap();
        let mut got = false;
        for _ in 0..200 {
            for (p, _) in next_packets(&mut ev, &mut rs, fam, Duration::from_millis(100)).await {
                if let Packet::Event(e) = p {
                    got |= e.number == event_number::WRIST_OFF;
                }
            }
            if got {
                break;
            }
        }
        assert!(got, "wrist off event");
        c.control("bogus", None).await.unwrap();
        let mut saw_err = false;
        for _ in 0..50 {
            if let Ok(Some(EmuMsg::Error { .. })) = timeout(Duration::from_millis(100), ev.recv()).await {
                saw_err = true;
                break;
            }
        }
        assert!(saw_err, "unknown control reports an error");
    }
}

#[tokio::test]
async fn out_of_range_and_busy() {
    let addr = spawn(DeviceFamily::Whoop4, 1.0, true, 5, 1).await;
    let (mut c, mut ev) = setup(&addr, DeviceFamily::Whoop4).await;
    // a second host is refused while the first is connected
    let err = EmulatorClient::connect(&addr).await.err().expect("busy");
    assert_eq!(err.kind(), std::io::ErrorKind::ConnectionRefused);

    c.control("out_of_range", None).await.unwrap();
    let mut disc = false;
    while let Ok(Some(m)) = timeout(Duration::from_secs(2), ev.recv()).await {
        if let EmuMsg::Disconnected { reason } = m {
            assert_eq!(reason, "out_of_range");
            disc = true;
            break;
        }
    }
    assert!(disc);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let err = EmulatorClient::connect(&addr).await.err().expect("out of range refused");
    assert_eq!(err.kind(), std::io::ErrorKind::NotConnected);
    let (mut ctl, adv) = EmulatorClient::connect_any(&addr).await.unwrap();
    assert!(matches!(adv, EmuMsg::Advertisement { in_range: false, .. }));
    ctl.control("in_range", None).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let (_c2, adv) = EmulatorClient::connect(&addr).await.unwrap();
    assert!(matches!(adv, EmuMsg::Advertisement { in_range: true, .. }));
}

async fn run_sync(c: &mut EmulatorClient, ev: &mut Receiver<EmuMsg>, fam: DeviceFamily, max_wait: Duration) -> (usize, bool) {
    let mut rs = ReassemblerSet::new(fam);
    let mut sync = HistorySync::new();
    c.write(fam.command_characteristic_uuid(), &HostCommand::SendHistoricalData.encode(fam, 9).unwrap()).await.unwrap();
    let (mut n, mut seq, mut done) = (0usize, 20u8, false);
    let deadline = tokio::time::Instant::now() + max_wait;
    while !done && tokio::time::Instant::now() < deadline {
        for (p, raw) in next_packets(ev, &mut rs, fam, Duration::from_millis(200)).await {
            match sync.on_packet(&p, &raw) {
                Some(SyncEvent::Chunk(ch)) => {
                    n += ch.records.len();
                    seq = seq.wrapping_add(1);
                    c.write(fam.command_characteristic_uuid(), &ch.ack().encode(fam, seq).unwrap()).await.unwrap();
                }
                Some(SyncEvent::Completed) => done = true,
                _ => {}
            }
        }
    }
    (n, done)
}

#[tokio::test]
async fn history_over_tcp_fast_and_throttled() {
    let fam = DeviceFamily::Whoop4;
    // fast: 1 day at 60 s cadence = ~1440 records, minus the deliberate gap(s)
    let addr = spawn(fam, 1.0, true, 60, 1).await;
    let (mut c, mut ev) = setup(&addr, fam).await;
    let (n, done) = run_sync(&mut c, &mut ev, fam, Duration::from_secs(20)).await;
    assert!(done, "sync complete");
    assert!((1300..=1440).contains(&n), "{n} records");
    // second sync: nothing left
    let (n2, done2) = run_sync(&mut c, &mut ev, fam, Duration::from_secs(5)).await;
    assert!(done2 && n2 <= 2, "acked data trimmed, got {n2}");

    // throttled default: a ~150 records/s trickle
    let addr = spawn(fam, 1.0, false, 60, 1).await;
    let (mut c, mut ev) = setup(&addr, fam).await;
    let (n, done) = run_sync(&mut c, &mut ev, fam, Duration::from_millis(1500)).await;
    assert!(!done && (20..700).contains(&n), "throttled: {n} records in 1.5 s");
}
