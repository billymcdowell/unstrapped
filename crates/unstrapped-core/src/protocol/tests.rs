//! Unit tests: CRC vectors, frame-integrity cases, golden packets, round trips.

use super::test_vectors::*;
use super::*;

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

const W4: DeviceFamily = DeviceFamily::Whoop4;
const W5: DeviceFamily = DeviceFamily::Whoop5;

fn dec(s: &str, f: DeviceFamily) -> Packet {
    let frame = Frame::decode(&hex(s), f).expect("valid frame");
    Packet::decode(&frame, f).expect("decodes")
}

// ---------------------------------------------------------------------------------------------
// CRC
// ---------------------------------------------------------------------------------------------

#[test]
fn crc_check_values() {
    // The standard "123456789" check strings for each catalogued algorithm.
    assert_eq!(crc8(b"123456789"), 0xF4); // CRC-8 (poly 0x07, init 0)
    assert_eq!(crc16_modbus(b"123456789"), 0x4B37); // CRC-16/MODBUS
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926); // CRC-32/ISO-HDLC (zlib)
    assert_eq!(crc32(&[]), 0);
}

#[test]
fn crc_frame_vectors() {
    // WHOOP 4 realtime frame: length bytes 18 00 -> crc8 0xff.
    assert_eq!(crc8(&[0x18, 0x00]), 0xFF);
    // WHOOP 5 CLIENT_HELLO header aa 01 08 00 00 01 -> crc16 0x71e6 (stored e6 71).
    assert_eq!(crc16_modbus(&[0xAA, 0x01, 0x08, 0x00, 0x00, 0x01]), 0x71E6);
    // ... and its body 23 01 91 01 -> crc32 8d5c3e36 (stored 36 3e 5c 8d).
    assert_eq!(crc32(&[0x23, 0x01, 0x91, 0x01]), 0x8D5C_3E36);
}

// ---------------------------------------------------------------------------------------------
// Frames
// ---------------------------------------------------------------------------------------------

#[test]
fn frame_integrity_cases() {
    for &(name, w5, h, want) in FRAME_CASES {
        let fam = if w5 { W5 } else { W4 };
        let r = Frame::decode(&hex(h), fam);
        let ok = matches!(
            (want, &r),
            ("ok", Ok(_))
                | ("no_sof", Err(FrameError::NoStartOfFrame))
                | ("short", Err(FrameError::TooShort { .. } | FrameError::DeclaredTooSmall { .. }))
                | ("length", Err(FrameError::LengthMismatch { .. }))
                | ("header_crc", Err(FrameError::HeaderCrc { .. }))
                | ("payload_crc", Err(FrameError::PayloadCrc { .. }))
        );
        assert!(ok, "{name}: expected {want}, got {r:?}");
    }
}

#[test]
fn whoop5_client_hello_is_reproduced() {
    let f = HostCommand::GetHello.encode(W5, 1).unwrap();
    assert_eq!(f, WHOOP5_CLIENT_HELLO.to_vec());
    assert_eq!(W5.client_hello().unwrap(), &WHOOP5_CLIENT_HELLO);
    assert!(W4.client_hello().is_none());
    let frame = Frame::decode(&f, W5).unwrap();
    assert_eq!((frame.packet_type, frame.seq, frame.cmd, frame.payload.as_slice()), (35, 1, 145, &[1][..]));
    assert_eq!(frame.header, Some([0x00, 0x01]));
}

/// WHOOP 5 envelope assembled by hand from the documented layout, independent of `Frame::encode`:
/// `AA 01 len16 00 01 crc16(header) inner(padded to 4) crc32(padded inner)`.
fn hand_built_w5_command(seq: u8, cmd: u8, body: &[u8]) -> Vec<u8> {
    let mut inner = vec![packet_type::COMMAND, seq, cmd];
    inner.extend_from_slice(body);
    while inner.len() % 4 != 0 {
        inner.push(0);
    }
    let len = ((inner.len() + 4) as u16).to_le_bytes();
    let mut b = vec![0xAA, 0x01, len[0], len[1], 0x00, 0x01];
    let h = crc16_modbus(&b).to_le_bytes();
    b.extend_from_slice(&h);
    b.extend_from_slice(&inner);
    b.extend_from_slice(&crc32(&inner).to_le_bytes());
    b
}

#[test]
fn whoop5_command_frames_match_hand_built_envelope() {
    let cases: [(u8, u8, Vec<u8>); 4] = [
        (1, 0x13, hex("012f98000000000000000000")),           // haptics, 12-byte body
        (1, 66, hex("040100a0c3b16a0f2f980000000000000000071e")), // SET_ALARM_TIME, 20-byte body
        (2, 68, vec![2, 1]),                                    // RUN_ALARM
        (1, 69, vec![2, 0xFF]),                                 // DISABLE_ALARM
    ];
    for (seq, cmd, body) in cases {
        let want = hand_built_w5_command(seq, cmd, &body);
        let f = Frame::new(packet_type::COMMAND, seq, cmd, body);
        assert_eq!(f.encode(W5).unwrap(), want, "cmd {cmd}");
        assert!(Frame::decode(&want, W5).is_ok());
    }
}

#[test]
fn whoop4_command_frame_layout() {
    // [AA][len][crc8][35][seq][cmd][body][crc32]; zero body command has length 7... here body is 1 byte.
    let b = HostCommand::GetBatteryLevel.encode(W4, 9).unwrap();
    assert_eq!(b.len(), 12);
    assert_eq!(b[0], 0xAA);
    assert_eq!(u16::from_le_bytes([b[1], b[2]]), 8);
    assert_eq!(b[3], crc8(&b[1..3]));
    assert_eq!(&b[4..8], &[35, 9, 26, 0]);
    assert_eq!(u32::from_le_bytes([b[8], b[9], b[10], b[11]]), crc32(&b[4..8]));
}

#[test]
fn frame_round_trip_both_families() {
    for fam in [W4, W5] {
        for plen in [0usize, 1, 2, 3, 4, 5, 17, 100, 1500] {
            let payload: Vec<u8> = (0..plen).map(|i| (i * 7 + 3) as u8).collect();
            let f = Frame::new(47, 24, 5, payload.clone());
            let bytes = f.encode(fam).unwrap();
            if fam == W5 {
                assert_eq!(bytes.len() % 4, 0, "padded to 4");
                assert_eq!(bytes.len(), 8 + (3 + plen).div_ceil(4) * 4 + 4);
            } else {
                assert_eq!(bytes.len(), 4 + 3 + plen + 4);
            }
            let back = Frame::decode(&bytes, fam).unwrap();
            assert_eq!((back.packet_type, back.seq, back.cmd), (47, 24, 5));
            assert_eq!(&back.payload[..plen], payload.as_slice());
            assert!(back.payload[plen..].iter().all(|&b| b == 0), "only zero padding may follow");
            // re-encoding the decoded frame is byte exact
            assert_eq!(back.encode(fam).unwrap(), bytes);
        }
    }
}

#[test]
fn frame_rejects_every_single_byte_corruption() {
    for fam in [W4, W5] {
        let good = Frame::new(40, 3, 1, vec![9, 8, 7, 6, 5, 4]).encode(fam).unwrap();
        for i in 0..good.len() {
            let mut bad = good.clone();
            bad[i] ^= 0x40;
            assert!(Frame::decode(&bad, fam).is_err(), "{fam:?} flip at {i} must be rejected");
        }
        // truncation and trailing byte
        assert!(matches!(
            Frame::decode(&good[..good.len() - 1], fam),
            Err(FrameError::LengthMismatch { .. })
        ));
        let mut extra = good.clone();
        extra.push(0);
        assert!(matches!(Frame::decode(&extra, fam), Err(FrameError::LengthMismatch { .. })));
    }
}

#[test]
fn frame_too_long_is_an_encode_error() {
    let f = Frame::new(47, 1, 1, vec![0; MAX_FRAME_BYTES]);
    assert!(matches!(f.encode(W4), Err(FrameError::TooLong { .. })));
    assert!(matches!(f.encode(W5), Err(FrameError::TooLong { .. })));
}

// ---------------------------------------------------------------------------------------------
// GATT
// ---------------------------------------------------------------------------------------------

#[test]
fn gatt_maps() {
    use CharacteristicRole::*;
    assert_eq!(W4.characteristics().len(), 4);
    assert_eq!(W5.characteristics().len(), 5);
    assert_eq!(W4.command_characteristic_uuid(), "61080002-8d6d-82b8-614a-1c8cb0f8dcc6");
    assert_eq!(W5.command_characteristic_uuid(), "fd4b0002-cce1-4033-93ce-002d5875f58a");
    assert_eq!(W4.role_of_uuid("61080005-8D6D-82B8-614A-1C8CB0F8DCC6"), Some(Data));
    assert_eq!(W5.role_of_uuid("fd4b0007-cce1-4033-93ce-002d5875f58a"), Some(Auxiliary));
    assert_eq!(W4.role_of_uuid("fd4b0002-cce1-4033-93ce-002d5875f58a"), None);
    assert_eq!(W4.notify_characteristics().count(), 3);
    assert_eq!(W5.notify_characteristics().count(), 4);
    assert_eq!(W4.header_crc_kind(), HeaderCrcKind::Crc8);
    assert_eq!(W5.header_crc_kind(), HeaderCrcKind::Crc16Modbus);
    assert_eq!(DeviceFamily::from_service_uuid(W5.service_uuid()), Some(W5));
}

// ---------------------------------------------------------------------------------------------
// Reassembler
// ---------------------------------------------------------------------------------------------

fn sample_frames(fam: DeviceFamily) -> Vec<Vec<u8>> {
    (0..6)
        .map(|i| {
            let payload: Vec<u8> = (0..(i * 90 + 5)).map(|j| (j * 13 + i) as u8).collect();
            Frame::new(47, 24, 5, payload).encode(fam).unwrap()
        })
        .collect()
}

fn run(fam: DeviceFamily, stream: &[u8], chunk: usize) -> (Vec<Vec<u8>>, ReassemblerStats) {
    let mut r = Reassembler::new(fam);
    let mut out = Vec::new();
    for c in stream.chunks(chunk) {
        for it in r.push(c) {
            if let Reassembled::Frame { raw, .. } = it {
                out.push(raw);
            }
        }
    }
    (out, r.stats())
}

#[test]
fn reassembler_any_fragment_size() {
    for fam in [W4, W5] {
        let frames = sample_frames(fam);
        let stream: Vec<u8> = frames.concat();
        for chunk in [1, 2, 3, 7, 20, 64, 244, 10_000] {
            let (got, st) = run(fam, &stream, chunk);
            assert_eq!(got, frames, "{fam:?} chunk {chunk}");
            assert_eq!(st.dropped_bytes, 0);
        }
    }
}

#[test]
fn reassembler_multiple_frames_in_one_notification_and_split_frames() {
    for fam in [W4, W5] {
        let a = Frame::new(40, 1, 0, vec![1, 2, 3]).encode(fam).unwrap();
        let b = Frame::new(48, 2, 3, vec![4, 5, 6, 7]).encode(fam).unwrap();
        let mut r = Reassembler::new(fam);
        let mut both = a.clone();
        both.extend_from_slice(&b);
        // both in one push
        assert_eq!(r.push_frames(&both).len(), 2);
        // second frame split at an awkward boundary together with the tail of the first
        let mut stream = a.clone();
        stream.extend_from_slice(&b);
        let cut = a.len() + 2;
        assert_eq!(r.push_frames(&stream[..cut]).len(), 1);
        assert_eq!(r.buffered_len(), 2);
        assert_eq!(r.push_frames(&stream[cut..]).len(), 1);
        assert_eq!(r.buffered_len(), 0);
    }
}

#[test]
fn reassembler_resyncs_after_garbage() {
    for fam in [W4, W5] {
        let frames = sample_frames(fam);
        let mut stream = vec![0x00, 0xAA, 0x13, 0x37, 0xAA, 0xAA, 0xFF];
        for f in &frames {
            stream.extend_from_slice(f);
            stream.extend_from_slice(&[0xAA, 0x01, 0x02]); // junk with false SOFs between frames
        }
        let (got, st) = run(fam, &stream, 11);
        assert_eq!(got, frames, "{fam:?}");
        assert!(st.dropped_bytes > 0);
    }
}

#[test]
fn reassembler_reports_corrupt_crc_and_recovers() {
    for fam in [W4, W5] {
        let frames = sample_frames(fam);
        let mut bad = frames[2].clone();
        let n = bad.len();
        bad[n - 10] ^= 0xFF; // payload corruption, header intact
        let mut stream = frames[0].clone();
        stream.extend_from_slice(&bad);
        stream.extend_from_slice(&frames[3]);
        let mut r = Reassembler::new(fam);
        let items = r.push(&stream);
        let good: Vec<_> = items
            .iter()
            .filter_map(|i| if let Reassembled::Frame { raw, .. } = i { Some(raw.clone()) } else { None })
            .collect();
        assert_eq!(good, vec![frames[0].clone(), frames[3].clone()]);
        let corrupt: Vec<_> = items
            .iter()
            .filter(|i| matches!(i, Reassembled::Corrupt { error: FrameError::PayloadCrc { .. }, .. }))
            .collect();
        assert_eq!(corrupt.len(), 1, "{fam:?}: the corrupt span is reported exactly once");
        if let Reassembled::Corrupt { raw, .. } = corrupt[0] {
            assert_eq!(raw, &bad);
        }
        assert_eq!(r.stats().corrupt_frames, 1);
    }
}

#[test]
fn reassembler_corrupt_header_does_not_swallow_following_frames() {
    for fam in [W4, W5] {
        let frames = sample_frames(fam);
        let mut bad = frames[1].clone();
        bad[2] ^= 0x10; // break the length / header
        let mut stream = bad;
        stream.extend_from_slice(&frames[4]);
        stream.extend_from_slice(&frames[5]);
        let (got, _) = run(fam, &stream, 9);
        assert_eq!(got, vec![frames[4].clone(), frames[5].clone()], "{fam:?}");
    }
}

#[test]
fn reassembler_set_routes_per_characteristic() {
    let f1 = Frame::new(40, 1, 0, vec![1, 2, 3]).encode(W4).unwrap();
    let f2 = Frame::new(48, 2, 3, vec![4, 5, 6]).encode(W4).unwrap();
    let mut set = ReassemblerSet::new(W4);
    // interleave halves of two frames arriving on two different characteristics
    let (a1, a2) = f1.split_at(5);
    let (b1, b2) = f2.split_at(6);
    assert!(set.push(CharacteristicRole::Data, a1).is_empty());
    assert!(set.push(CharacteristicRole::Events, b1).is_empty());
    assert_eq!(set.push(CharacteristicRole::Data, a2).len(), 1);
    let r = set.push_uuid("61080004-8d6d-82b8-614a-1c8cb0f8dcc6", b2).unwrap();
    assert_eq!(r.len(), 1);
    assert!(set.push_uuid("deadbeef-0000-0000-0000-000000000000", &f1).is_none());
}

#[test]
fn fragment_helper() {
    let frame: Vec<u8> = (0..100u8).collect();
    let parts = fragment(&frame, 23); // 20 bytes per notification
    assert_eq!(parts.len(), 5);
    assert!(parts.iter().all(|p| p.len() == 20));
    assert_eq!(parts.concat(), frame);
    assert_eq!(fragment(&frame, 247).len(), 1);
    assert_eq!(fragment_by(&frame, 7).len(), 15);
    assert_eq!(fragment(&frame, 0).concat(), frame); // degenerate MTU still progresses
    assert!(fragment(&[], 23).is_empty());
}

// ---------------------------------------------------------------------------------------------
// Golden packets (synthetic, see test_vectors.rs)
// ---------------------------------------------------------------------------------------------

#[test]
fn realtime_hr_golden_frames() {
    let Packet::RealtimeHr(r) = dec(W4_REALTIME, W4) else { panic!() };
    assert_eq!((r.bpm, r.rr_ms.as_slice(), r.device_seconds, r.subsec), (62, &[968][..], 1_790_000_000, 0x0140));
    assert_eq!(Packet::RealtimeHr(r).encode(W4).unwrap(), hex(W4_REALTIME));

    let Packet::RealtimeHr(r) = dec(W5_REALTIME, W5) else { panic!() };
    assert_eq!((r.bpm, r.device_seconds), (91, 1_790_000_050));
    assert_eq!(r.rr_ms, vec![655, 662]); // default: the words are milliseconds (U3)
    assert_eq!(r.unix_ms(), 1_790_000_050_000);

    // opt-in tick interpretation
    let opts = CodecOptions { whoop5_rr: RrUnit::Ticks1024 };
    let frame = Frame::decode(&hex(W5_REALTIME), W5).unwrap();
    let Packet::RealtimeHr(t) = Packet::decode_with(&frame, W5, &opts).unwrap() else { panic!() };
    assert_eq!(t.rr_ms, vec![640, 646]); // (655*1000+512)/1024, (662*1000+512)/1024
}

#[test]
fn events_golden_frames() {
    let Packet::Event(e) = dec(W5_BATTERY_EVENT, W5) else { panic!() };
    assert_eq!((e.number, e.unix), (event_number::BATTERY_LEVEL, 1_790_000_100));
    let b = e.as_battery().unwrap();
    assert_eq!((b.percent, b.millivolts, b.charging), (63.4, 3912, false));

    let Packet::Event(e) = dec(W5_DOUBLE_TAP, W5) else { panic!() };
    assert_eq!((e.number, e.unix), (event_number::DOUBLE_TAP, 1_790_000_200));
    assert_eq!(event_number::name(e.number), Some("DOUBLE_TAP"));
    assert!(e.as_battery().is_none());

    let Packet::Event(e) = dec(W4_EVENT_WRIST_ON, W4) else { panic!() };
    assert_eq!((e.number, e.unix), (event_number::WRIST_ON, 1_790_000_300));

    let Packet::Event(e) = dec(W4_EVENT_BATTERY, W4) else { panic!() };
    let b = e.as_battery().unwrap();
    assert_eq!((b.percent, b.millivolts, b.charging), (71.5, 3987, true));
}

#[test]
fn command_responses_golden_frames() {
    let Packet::CommandResponse(r) = dec(W5_BATTERY_RESPONSE, W5) else { panic!() };
    assert_eq!((r.cmd, r.origin_seq, r.result), (26, 5, CommandResult::Success));
    assert_eq!(r.typed_body(W5).unwrap(), ResponseBody::Battery { percent: 81.0 });

    let Packet::CommandResponse(r) = dec(W4_BATTERY_RESPONSE, W4) else { panic!() };
    assert_eq!(r.typed_body(W4).unwrap(), ResponseBody::Battery { percent: 64.5 });

    let Packet::CommandResponse(r) = dec(W4_DATA_RANGE, W4) else { panic!() };
    assert_eq!((r.cmd, r.origin_seq, r.result), (34, 0x0B, CommandResult::Success));
    let ResponseBody::DataRange(d) = r.typed_body(W4).unwrap() else { panic!() };
    assert_eq!((d.read_page, d.write_page, d.trim_page, d.capacity_pages), (52_010, 52_100, 52_010, 131_072));
    assert_eq!(d.record_estimate, 1_350);
    assert_eq!(d.pages_behind(), Some(90));
    assert_eq!(d.newest_unix(), Some(1_790_000_000));
    assert_eq!(d.oldest_unix(), Some(1_789_990_000));
    assert_eq!(d.clocks[0].0, 1_262_044_800); // sentinel, not a real time
    assert_eq!(DataRange::plausible_unix(d.clocks[0].0), None);
    let rebuilt = Packet::CommandResponse(CommandResponse {
        body: ResponseBody::DataRange(d).encode(W4),
        ..r.clone()
    });
    assert_eq!(rebuilt.encode(W4).unwrap(), hex(W4_DATA_RANGE));

    let Packet::CommandResponse(r) = dec(W5_DATA_RANGE, W5) else { panic!() };
    let ResponseBody::DataRange(d) = r.typed_body(W5).unwrap() else { panic!() };
    assert_eq!((d.boundary_a, d.read_page, d.write_page, d.trim_page, d.wrap), (98_000, 97_010, 97_400, 97_010, 16));
    assert_eq!(d.pages_behind(), Some(390));
    assert_eq!(d.newest_unix(), Some(1_790_001_000));
    let rebuilt = Packet::CommandResponse(CommandResponse {
        body: ResponseBody::DataRange(d).encode(W5),
        ..r.clone()
    });
    assert_eq!(rebuilt.encode(W5).unwrap(), hex(W5_DATA_RANGE));
}

#[test]
fn history_metadata_golden_frames() {
    let Packet::HistoryMetadata(HistoryMetadata::Start { payload, .. }) = dec(W4_META_START, W4) else { panic!() };
    assert!(payload.is_empty());
    assert!(matches!(dec(W4_META_COMPLETE, W4), Packet::HistoryMetadata(HistoryMetadata::Complete { .. })));

    let Packet::HistoryMetadata(HistoryMetadata::End { end, seq }) = dec(W4_META_END, W4) else { panic!() };
    assert_eq!((end.unix, end.trim_cursor(), seq), (1_790_000_000, 54_321, 0));
    let again = Packet::HistoryMetadata(HistoryMetadata::End { seq, end });
    assert_eq!(again.encode(W4).unwrap(), hex(W4_META_END));

    let Packet::HistoryMetadata(HistoryMetadata::End { end, seq }) = dec(W5_META_END, W5) else { panic!() };
    assert_eq!((end.unix, end.trim_cursor()), (1_790_001_000, 97_010));
    assert_eq!(end.ack_token, hex("f27a010010000000"));
    let again = Packet::HistoryMetadata(HistoryMetadata::End { seq, end });
    assert_eq!(again.encode(W5).unwrap(), hex(W5_META_END));
}

fn golden(name: &str) -> &'static str {
    HISTORY.iter().find(|(n, _)| *n == name).unwrap().1
}

#[test]
fn history_whoop4_v24_golden() {
    let Packet::Historical(HistoricalPacket::Record(r)) = dec(golden("w4_v24_worn"), W4) else { panic!() };
    assert_eq!((r.version, r.index), (24, 40_100_200));
    let s = &r.sample;
    assert_eq!(s.unix_ms, 1_790_000_500_000);
    assert_eq!(s.bpm, Some(72));
    assert_eq!(s.rr_ms, vec![812, 833]);
    assert_eq!(s.skin_temp_raw, Some(880));
    assert_eq!(s.spo2, Some(Spo2Raw { red: 610, ir: 640 }));
    let g = s.motion.unwrap().gravity.unwrap();
    assert!((g[0] + 0.25).abs() < 1e-6 && (g[1] - 0.5).abs() < 1e-6 && (g[2] - 0.82).abs() < 1e-6);
    // provisional temperature map: raw 880 -> 33 + 54*0.05
    assert!((s.skin_temp_c.unwrap() - 35.7).abs() < 1e-4);
}

#[test]
fn history_whoop4_v25_golden() {
    for (n, secs, idx) in [("w4_v25_a", 1_790_000_600u32, 31_000u32), ("w4_v25_b", 1_790_000_601, 31_001), ("w4_v25_c", 1_790_000_602, 31_002)] {
        let Packet::Historical(HistoricalPacket::Record(r)) = dec(golden(n), W4) else { panic!() };
        assert_eq!((r.version, r.index, r.sample.unix_ms), (25, idx, secs as i64 * 1000), "{n}");
        assert_eq!(r.sample.bpm, None);
        let g = r.sample.motion.unwrap().gravity.unwrap();
        let mag = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt();
        assert!((0.9..1.1).contains(&mag), "{n}: |g| = {mag}"); // a strap at rest reads about 1 g
    }
    // gravity is stored as i16 / 16384, so it survives to about 1e-4
    let Packet::Historical(HistoricalPacket::Record(r)) = dec(golden("w4_v25_a"), W4) else { panic!() };
    let g = r.sample.motion.unwrap().gravity.unwrap();
    assert!((g[0] - 0.10).abs() < 1e-4 && (g[1] + 0.20).abs() < 1e-4 && (g[2] - 0.97).abs() < 1e-4);
}

#[test]
fn history_whoop5_r18_golden() {
    let Packet::Historical(HistoricalPacket::Record(r)) = dec(golden("w5_r18_worn"), W5) else { panic!() };
    let s = &r.sample;
    assert_eq!((r.version, r.marker, r.index), (18, 0x80, 30_000_100));
    assert_eq!((s.unix_ms, s.bpm), (1_790_000_700_000, Some(84)));
    assert_eq!(s.rr_ms, vec![702, 715]);
    assert_eq!(s.skin_temp_raw, Some(3312));
    assert!((s.skin_temp_c.unwrap() - 33.12).abs() < 1e-4);
    assert_eq!(s.steps, Some(120));
    assert_eq!(s.motion.unwrap().activity, Some(1));

    let Packet::Historical(HistoricalPacket::Record(r)) = dec(golden("w5_r18_one_rr"), W5) else { panic!() };
    assert_eq!((r.sample.bpm, r.sample.rr_ms.clone(), r.sample.unix_ms), (Some(83), vec![721], 1_790_000_702_000));
    assert!((r.sample.motion.unwrap().dynamic_accel_g.unwrap() - 0.0125).abs() < 1e-6);

    // off wrist: HR 0 means no reading, temperature 23.10 C
    let Packet::Historical(HistoricalPacket::Record(r)) = dec(golden("w5_r18_offwrist"), W5) else { panic!() };
    assert_eq!((r.sample.bpm, r.sample.skin_temp_raw), (None, Some(2310)));
    assert!(r.sample.rr_ms.is_empty());

    let Packet::Historical(HistoricalPacket::Record(r)) = dec(golden("w5_r18_resting"), W5) else { panic!() };
    assert_eq!((r.sample.bpm, r.sample.skin_temp_raw, r.sample.unix_ms), (Some(58), Some(3365), 1_790_050_000_000));
    let g = r.sample.motion.unwrap().gravity.unwrap();
    assert!(((g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt() - 1.0).abs() < 0.01);

    let Packet::Historical(HistoricalPacket::Record(r)) = dec(golden("w5_r18_low_hr"), W5) else { panic!() };
    assert_eq!((r.sample.bpm, r.sample.skin_temp_raw, r.sample.rr_ms.clone()), (Some(49), Some(3021), vec![1220]));
}

#[test]
fn history_goldens_re_encode_byte_for_byte() {
    for (name, h) in HISTORY {
        let fam = if name.starts_with("w5") { W5 } else { W4 };
        let p = dec(h, fam);
        assert_eq!(p.encode(fam).unwrap(), hex(h), "{name}");
    }
}

#[test]
fn unknown_history_layouts_are_preserved_not_guessed() {
    // WHOOP 4 version 99, WHOOP 5 layout 20 (R20 optical): both stay opaque.
    for (fam, ver) in [(W4, 99u8), (W5, 20u8)] {
        let frame = Frame::new(packet_type::HISTORICAL_DATA, ver, 0x80, vec![1; 200]);
        let bytes = frame.encode(fam).unwrap();
        let p = Packet::decode(&Frame::decode(&bytes, fam).unwrap(), fam).unwrap();
        let Packet::Historical(HistoricalPacket::Unmapped { version, frame }) = p else { panic!() };
        assert_eq!(version, ver);
        assert_eq!(frame.encode(fam).unwrap(), bytes);
    }
    // a v24 frame that is too short to hold the map is also opaque
    let short = Frame::new(47, 24, 5, vec![0; 20]);
    assert!(matches!(
        Packet::decode(&short, W4).unwrap(),
        Packet::Historical(HistoricalPacket::Unmapped { .. })
    ));
}

#[test]
fn other_packet_types_pass_through() {
    // type 43 raw data and 50 console logs are preserved
    for t in [43u8, 50, 51, 52] {
        let f = Frame::new(t, 1, 2, vec![3, 4, 5, 6]);
        assert!(matches!(Packet::decode(&f, W4).unwrap(), Packet::Unknown(_)));
    }
}

// ---------------------------------------------------------------------------------------------
// Round trips: encode -> fragment -> reassemble -> decode, for every packet kind
// ---------------------------------------------------------------------------------------------

fn full_sample(unix: i64) -> HistorySample {
    HistorySample {
        unix_ms: unix * 1000,
        bpm: Some(63),
        rr_ms: vec![950, 962, 940, 955],
        skin_temp_c: Some(33.25),
        skin_temp_raw: None,
        motion: Some(Motion { gravity: Some([0.125, -0.5, 0.8300781]), dynamic_accel_g: Some(0.03125), activity: Some(1) }),
        spo2: Some(Spo2Raw { red: 18000, ir: 17000 }),
        steps: Some(1234),
    }
}

/// A sample the family's primary layout can carry losslessly.
fn exact_sample(fam: DeviceFamily, unix: i64) -> HistorySample {
    let mut s = full_sample(unix);
    match fam {
        W4 => {
            s.steps = None;
            s.motion = s.motion.map(|m| Motion { dynamic_accel_g: None, activity: None, ..m });
            s.skin_temp_raw = Some(861);
            s.skin_temp_c = Some(history::whoop4_skin_temp_c(861));
        }
        W5 => {
            s.spo2 = None;
            s.skin_temp_raw = Some(3325);
            s.skin_temp_c = Some(3325f32 / 100.0);
        }
    }
    s
}

fn all_packets(fam: DeviceFamily) -> Vec<Packet> {
    let mut v = Vec::new();
    v.push(Packet::Command(CommandFrame { seq: 4, cmd: 66, payload: vec![1, 2, 3, 4, 5] }));
    for (cmd, result, body) in [
        (26u8, CommandResult::Success, ResponseBody::Battery { percent: 87.0 }.encode(fam)),
        (34, CommandResult::Pending, vec![]),
        (34, CommandResult::Success, ResponseBody::DataRange(sample_range()).encode(fam)),
        (if fam == W5 { 147 } else { 11 }, CommandResult::Success, ResponseBody::Clock(ClockReading { unix: 1_780_000_000, subsec_ticks: (fam == W5).then_some(1234) }).encode(fam)),
        (3, CommandResult::Unsupported, vec![]),
        (22, CommandResult::Other(9), vec![]),
    ] {
        v.push(Packet::CommandResponse(CommandResponse { seq: 77, cmd, origin_seq: 5, result, body }));
    }
    v.push(Packet::RealtimeHr(RealtimeHr { seq: 9, device_seconds: 1_780_000_123, subsec: 321, bpm: 71, rr_ms: vec![845, 850, 840] }));
    v.push(Packet::RealtimeHr(RealtimeHr { seq: 10, device_seconds: 1_780_000_124, subsec: 0, bpm: 0, rr_ms: vec![] }));
    v.push(Packet::Event(Event::simple(3, event_number::WRIST_ON, 1_780_000_000)));
    v.push(Packet::Event(Event::battery(4, 1_780_000_060, BatteryEvent { percent: 76.5, millivolts: 3987, charging: true })));
    v.push(Packet::HistoryMetadata(HistoryMetadata::Start { seq: 1, payload: if fam == W5 { vec![1; 12] } else { vec![] } }));
    v.push(Packet::HistoryMetadata(HistoryMetadata::End {
        seq: 2,
        end: HistoryEnd { unix: 1_780_000_200, subsec: 77, reserved: 5, ack_token: hex("41b6010010000000") },
    }));
    v.push(Packet::HistoryMetadata(HistoryMetadata::Complete { seq: 3, payload: vec![] }));
    let rec = HistoryRecord::new(fam, 4242, exact_sample(fam, 1_780_000_000));
    v.push(rec.into());
    if fam == W4 {
        // other encodable WHOOP 4 layouts
        let mut s = full_sample(1_780_000_001);
        s.steps = None;
        s.spo2 = None;
        let mut r25 = HistoryRecord::new(W4, 9, HistorySample { unix_ms: s.unix_ms, motion: Some(Motion { gravity: Some([0.25, 0.5, -0.75]), ..Motion::default() }), ..Default::default() });
        r25.version = 25;
        r25.marker = 0;
        v.push(r25.into());
        let mut r5 = HistoryRecord::new(W4, 10, HistorySample { unix_ms: s.unix_ms, bpm: Some(55), rr_ms: vec![1000], ..Default::default() });
        r5.version = 5;
        v.push(r5.into());
    }
    v.push(Packet::Unknown(Frame::new(50, 0, 0, b"console line\0".to_vec())));
    v
}

fn sample_range() -> DataRange {
    DataRange {
        boundary_a: 1,
        read_page: 100,
        write_page: 400,
        trim_page: 100,
        wrap: 2,
        capacity_pages: 131_072,
        record_estimate: 4500,
        derived_backlog: 4500,
        clocks: [(1_262_044_800, 0), (1_780_000_000, 5), (1_780_000_000, 5), (1_780_003_600, 99)],
    }
}

/// Compare a decoded packet with the original, allowing for WHOOP 5 zero padding on opaque bodies
/// and for the documented derived/rounded fields.
fn assert_same(fam: DeviceFamily, a: &Packet, b: &Packet) {
    match (a, b) {
        (Packet::Command(x), Packet::Command(y)) => {
            assert_eq!((x.seq, x.cmd), (y.seq, y.cmd));
            assert_eq!(&y.payload[..x.payload.len()], x.payload.as_slice());
            assert!(y.payload[x.payload.len()..].iter().all(|&z| z == 0));
        }
        (Packet::CommandResponse(x), Packet::CommandResponse(y)) => {
            assert_eq!((x.seq, x.cmd, x.origin_seq, x.result), (y.seq, y.cmd, y.origin_seq, y.result));
            assert_eq!(&y.body[..x.body.len()], x.body.as_slice());
            assert!(y.body[x.body.len()..].iter().all(|&z| z == 0));
        }
        (Packet::Event(x), Packet::Event(y)) => {
            assert_eq!((x.seq, x.number, x.unix), (y.seq, y.number, y.unix));
            assert_eq!(&y.payload[..x.payload.len()], x.payload.as_slice());
        }
        (Packet::HistoryMetadata(HistoryMetadata::Start { payload: x, seq: s1 }), Packet::HistoryMetadata(HistoryMetadata::Start { payload: y, seq: s2 }))
        | (Packet::HistoryMetadata(HistoryMetadata::Complete { payload: x, seq: s1 }), Packet::HistoryMetadata(HistoryMetadata::Complete { payload: y, seq: s2 })) => {
            assert_eq!(s1, s2);
            assert_eq!(&y[..x.len()], x.as_slice());
        }
        (Packet::Unknown(x), Packet::Unknown(y)) => {
            // the WHOOP 5 header flag bytes are filled in on the wire
            assert_eq!((x.packet_type, x.seq, x.cmd, &x.payload), (y.packet_type, y.seq, y.cmd, &y.payload));
        }
        _ => assert_eq!(a, b, "{fam:?}"),
    }
}

#[test]
fn every_packet_round_trips_through_fragmentation() {
    for fam in [W4, W5] {
        for mtu in [23usize, 185, 517] {
            let packets = all_packets(fam);
            // encode, fragment and interleave into one stream of notifications
            let mut notifications: Vec<Vec<u8>> = Vec::new();
            for p in &packets {
                let bytes = p.encode(fam).unwrap();
                notifications.extend(fragment(&bytes, mtu));
            }
            let mut r = Reassembler::new(fam);
            let mut decoded = Vec::new();
            for n in &notifications {
                for f in r.push_frames(n) {
                    decoded.push(Packet::decode(&f, fam).unwrap());
                }
            }
            assert_eq!(decoded.len(), packets.len(), "{fam:?} mtu {mtu}");
            for (a, b) in packets.iter().zip(&decoded) {
                assert_same(fam, a, b);
            }
            assert_eq!(r.stats().dropped_bytes, 0);
        }
    }
}

#[test]
fn history_sample_round_trip_exact_values() {
    for fam in [W4, W5] {
        let sample = full_sample(1_780_123_456);
        let rec = HistoryRecord::new(fam, 7, sample.clone());
        let bytes = Packet::from(rec).encode(fam).unwrap();
        let Packet::Historical(HistoricalPacket::Record(back)) =
            Packet::decode(&Frame::decode(&bytes, fam).unwrap(), fam).unwrap()
        else {
            panic!()
        };
        let mut want = sample;
        // fields the layout cannot carry are dropped
        match fam {
            W4 => {
                want.steps = None;
                want.motion = want.motion.map(|m| Motion { dynamic_accel_g: None, activity: None, ..m });
                want.rr_ms.truncate(5);
                want.skin_temp_raw = back.sample.skin_temp_raw; // raw derived from the provisional map
                want.skin_temp_c = back.sample.skin_temp_c;
                assert!((back.sample.skin_temp_c.unwrap() - 33.25).abs() < 0.03);
            }
            W5 => {
                want.spo2 = None;
                want.skin_temp_raw = Some(3325);
            }
        }
        assert_eq!(back.sample, want, "{fam:?}");
        assert_eq!(back.index, 7);
    }
}

#[test]
fn rr_unit_round_trips_to_within_a_millisecond() {
    for ms in 1u16..=3000 {
        let back = RrUnit::Ticks1024.to_ms(RrUnit::Ticks1024.from_ms(ms));
        assert!((back as i32 - ms as i32).abs() <= 1, "{ms} -> {back}");
        assert_eq!(RrUnit::Milliseconds.to_ms(RrUnit::Milliseconds.from_ms(ms)), ms);
    }
    // W5 realtime / history with tick unit round-trips within 1 ms
    let opts = CodecOptions { whoop5_rr: RrUnit::Ticks1024 };
    let p = Packet::RealtimeHr(RealtimeHr { seq: 1, device_seconds: 1_780_000_000, subsec: 0, bpm: 60, rr_ms: vec![1000, 812] });
    let f = Frame::decode(&p.encode_with(W5, &opts).unwrap(), W5).unwrap();
    let Packet::RealtimeHr(r) = Packet::decode_with(&f, W5, &opts).unwrap() else { panic!() };
    assert!(r.rr_ms.iter().zip([1000u16, 812]).all(|(a, b)| (*a as i32 - b as i32).abs() <= 1));
}

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

fn all_commands() -> Vec<HostCommand> {
    vec![
        HostCommand::ToggleRealtimeHr(true),
        HostCommand::ToggleRealtimeHr(false),
        HostCommand::GetBatteryLevel,
        HostCommand::GetClock,
        HostCommand::SetClock { unix: 1_780_000_000, subsec_ticks: 0 },
        HostCommand::GetDataRange,
        HostCommand::SendHistoricalData,
        HostCommand::HistoricalDataResult { ack_token: hex("41b6010010000000") },
        HostCommand::AbortHistoricalTransmits,
        HostCommand::GetHello,
        HostCommand::Raw { cmd: 84, payload: vec![0] },
    ]
}

#[test]
fn every_host_command_round_trips() {
    for fam in [W4, W5] {
        let mut cmds = all_commands();
        if fam == W5 {
            cmds.push(HostCommand::SetClock { unix: 1_780_000_000, subsec_ticks: 16384 });
        } else {
            cmds.push(HostCommand::GetVersionInfo);
            cmds.push(HostCommand::GetExtendedBatteryInfo);
        }
        for c in cmds {
            let bytes = c.encode(fam, 5).unwrap();
            let frame = Frame::decode(&bytes, fam).unwrap();
            assert_eq!(frame.packet_type, packet_type::COMMAND);
            assert_eq!(frame.seq, 5);
            let back = HostCommand::from_frame(&frame, fam).unwrap();
            match (&c, &back) {
                // WHOOP 5 pads Raw bodies with zeros
                (HostCommand::Raw { cmd: a, payload: p }, HostCommand::Raw { cmd: b, payload: q }) => {
                    assert_eq!(a, b);
                    assert_eq!(&q[..p.len()], p.as_slice());
                }
                _ => assert_eq!(c, back, "{fam:?}"),
            }
            // and the generic packet decoder sees a command
            assert!(matches!(Packet::decode(&frame, fam).unwrap(), Packet::Command(_)));
        }
    }
    assert!(matches!(HostCommand::GetVersionInfo.encode(W5, 1), Err(PacketError::Unsupported(_))));
    assert!(matches!(HostCommand::GetExtendedBatteryInfo.encode(W5, 1), Err(PacketError::Unsupported(_))));
}

#[test]
fn command_numbers_and_bodies() {
    assert_eq!(HostCommand::GetClock.command_number(W4).unwrap(), 11);
    assert_eq!(HostCommand::GetClock.command_number(W5).unwrap(), 147);
    assert_eq!(HostCommand::SetClock { unix: 0, subsec_ticks: 0 }.command_number(W4).unwrap(), 10);
    assert_eq!(HostCommand::SetClock { unix: 0, subsec_ticks: 0 }.command_number(W5).unwrap(), 146);
    assert_eq!(HostCommand::GetHello.command_number(W4).unwrap(), 35);
    assert_eq!(HostCommand::GetHello.command_number(W5).unwrap(), 145);
    assert_eq!(HostCommand::SendHistoricalData.command_number(W4).unwrap(), 22);
    assert_eq!(HostCommand::HistoricalDataResult { ack_token: vec![] }.command_number(W5).unwrap(), 23);
    assert_eq!(HostCommand::SetClock { unix: 0x0403_0201, subsec_ticks: 0 }.payload(W4), vec![1, 2, 3, 4, 0, 0, 0, 0]);
    assert_eq!(HostCommand::SetClock { unix: 0x0403_0201, subsec_ticks: 0x0605 }.payload(W5), vec![1, 1, 2, 3, 4, 5, 6]);
    let ack = HostCommand::HistoricalDataResult { ack_token: vec![9; 8] }.payload(W4);
    assert_eq!(ack, [vec![1], vec![9; 8]].concat());
}

#[test]
fn seq_counter_wraps() {
    let mut s = SeqCounter::new(254);
    assert_eq!((s.next_seq(), s.next_seq(), s.next_seq()), (254, 255, 0));
}

#[test]
fn version_info_and_extended_battery_bodies() {
    let v = VersionInfo { harvard: [41, 17, 6, 0], boylston: [17, 2, 2, 0] };
    let body = ResponseBody::VersionInfo(v).encode(W4);
    assert_eq!(body.len(), 68);
    assert_eq!(ResponseBody::decode(7, W4, &body).unwrap(), ResponseBody::VersionInfo(v));
    let eb = ResponseBody::ExtendedBattery { millivolts: 3990 };
    assert_eq!(ResponseBody::decode(98, W4, &eb.encode(W4)).unwrap(), eb);
    // truncated bodies are typed errors, not panics
    assert!(matches!(ResponseBody::decode(26, W4, &[1]), Err(PacketError::Truncated { .. })));
    assert!(matches!(ResponseBody::decode(34, W5, &[1; 10]), Err(PacketError::Truncated { .. })));
}

#[test]
fn truncated_packets_are_errors_not_panics() {
    for fam in [W4, W5] {
        for t in [36u8, 40, 48, 49] {
            let f = Frame::new(t, 1, 2, vec![]);
            // 'ok or error', never a panic; metadata END (cmd 2) needs a payload
            let _ = Packet::decode(&f, fam);
        }
        assert!(matches!(
            Packet::decode(&Frame::new(49, 1, meta_type::HISTORY_END, vec![0; 5]), fam),
            Err(PacketError::Truncated { .. })
        ));
        assert!(matches!(
            Packet::decode(&Frame::new(36, 1, 26, vec![0]), fam),
            Err(PacketError::Truncated { .. })
        ));
    }
}

// ---------------------------------------------------------------------------------------------
// History sync
// ---------------------------------------------------------------------------------------------

#[test]
fn history_sync_persist_before_ack() {
    for fam in [W4, W5] {
        let token = hex("41b6010010000000");
        let mk = |p: Packet| {
            let bytes = p.encode(fam).unwrap();
            (Packet::decode(&Frame::decode(&bytes, fam).unwrap(), fam).unwrap(), bytes)
        };
        let mut sync = HistorySync::new();
        let (start, b) = mk(Packet::HistoryMetadata(HistoryMetadata::Start { seq: 1, payload: vec![] }));
        assert_eq!(sync.on_packet(&start, &b), Some(SyncEvent::Started));
        let mut raws = Vec::new();
        for i in 0..3u32 {
            let (rec, b) = mk(HistoryRecord::new(fam, i, full_sample(1_780_000_000 + i as i64)).into());
            raws.push(b.clone());
            assert_eq!(sync.on_packet(&rec, &b), None);
        }
        let (end, b) = mk(Packet::HistoryMetadata(HistoryMetadata::End {
            seq: 2,
            end: HistoryEnd { unix: 1_780_000_003, subsec: 0, reserved: 0, ack_token: token.clone() },
        }));
        let Some(SyncEvent::Chunk(chunk)) = sync.on_packet(&end, &b) else { panic!() };
        assert_eq!(chunk.records.len(), 3);
        assert_eq!(chunk.raw_frames, raws);
        // ... the application persists `chunk` here ...
        let ack = chunk.ack();
        assert_eq!(ack, HostCommand::HistoricalDataResult { ack_token: token });
        // an empty END is still a chunk to acknowledge
        let Some(SyncEvent::Chunk(c2)) = sync.on_packet(&end, &b) else { panic!() };
        assert!(c2.records.is_empty());
        let (done, b) = mk(Packet::HistoryMetadata(HistoryMetadata::Complete { seq: 3, payload: vec![] }));
        assert_eq!(sync.on_packet(&done, &b), Some(SyncEvent::Completed));
        assert!(!sync.is_active());
    }
}

// ---------------------------------------------------------------------------------------------
// Standard Heart Rate Measurement
// ---------------------------------------------------------------------------------------------

#[test]
fn standard_heart_rate_measurement() {
    // flags 0x10 (RR present), hr 72, one RR of 1024 units = 1000 ms
    let hr = StdHeartRate::decode(&[0x10, 72, 0x00, 0x04]).unwrap();
    assert_eq!((hr.bpm, hr.rr_ms.as_slice(), hr.contact, hr.energy_kj), (72, &[1000][..], None, None));
    let full = StdHeartRate { bpm: 300, contact: Some(true), energy_kj: Some(5), rr_ms: vec![800, 810] };
    let back = StdHeartRate::decode(&full.encode()).unwrap();
    assert_eq!((back.bpm, back.contact, back.energy_kj), (300, Some(true), Some(5)));
    assert!(back.rr_ms.iter().zip(&full.rr_ms).all(|(a, b)| (*a as i32 - *b as i32).abs() <= 1));
    let plain = StdHeartRate { bpm: 60, contact: None, energy_kj: None, rr_ms: vec![] };
    assert_eq!(plain.encode(), vec![0, 60]);
    assert_eq!(StdHeartRate::decode(&plain.encode()).unwrap(), plain);
    assert!(StdHeartRate::decode(&[]).is_err());
    assert!(StdHeartRate::decode(&[1, 60]).is_err());
}
