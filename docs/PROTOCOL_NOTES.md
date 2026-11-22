# WHOOP BLE protocol notes

Working notes for `crates/unstrapped-core/src/protocol`. Facts (UUIDs, offsets, CRC parameters, command
numbers) come from the NOOP reference documentation (https://github.com/ryanbr/noop), which states that
these protocol facts are free to reuse; the Rust code is written independently and none of NOOP's code
or fixture files are included here. "Verified" below means: during development the layout was checked
against a real captured frame that NOOP publishes. The test vectors in this repository are synthetic
(see section 6). Nothing here has been run against a physical strap by us.

Offsets are **inner-record offsets** unless stated: `inner[0]` is the packet type, `inner[1]` the
sequence/version byte, `inner[2]` the command/event/marker byte, payload from `inner[3]`. Add 4 for
an absolute WHOOP 4 frame offset, 8 for WHOOP 5. All integers are little-endian.

## 1. GATT

| | WHOOP 4.0 | WHOOP 5.0 / MG |
|---|---|---|
| Service | `61080001-8d6d-82b8-614a-1c8cb0f8dcc6` | `fd4b0001-cce1-4033-93ce-002d5875f58a` |
| Command write | `…0002` (`61080002-8d6d-82b8-614a-1c8cb0f8dcc6`) | `fd4b0002-cce1-4033-93ce-002d5875f58a` |
| Command-response notify | `…0003` | `fd4b0003-…` (role by analogy) |
| Event notify | `…0004` | `fd4b0004-…` (role by analogy) |
| Data notify (fragmented) | `…0005` | `fd4b0005-…` (role by analogy) |
| Extra notify | none | `fd4b0007-…` (purpose undocumented) |
| Header check | CRC-8 | CRC-16/MODBUS |
| Session start | confirmed write of `GET_BATTERY_LEVEL` | write fixed `CLIENT_HELLO` immediately |

Both families also expose standard Heart Rate (`180D`/`2A37`, works unbonded) and Battery
(`180F`/`2A19`; some WHOOP 4 units report a constant 100, so prefer command 26). Subscribe to every
notify characteristic and route by packet type; keep one reassembler per characteristic.

WHOOP 5 also advertises other service families (`11500001-…`, `8a580001-…`, `59830001-…`); their
framing is unmapped and they must not be connected to.

## 2. Frame envelope

```
WHOOP 4                                      WHOOP 5 / MG (format 1)
[0]    AA                                    [0]    AA
[1..3) length = inner + 4                    [1]    01
[3]    crc8(frame[1..3])                     [2..4) declared length = padded inner + 4
[4..)  inner                                 [4..6) flags: host->strap 00 01, strap->host 01 00
[len..len+4) crc32(inner)                    [6..8) crc16-modbus(frame[0..6])
total = length + 4                           [8..)  inner, zero padded to a multiple of 4
                                             [end-4..end) crc32(padded inner)
                                             total = declared length + 8
```

* CRC-8: poly `07`, init `00`, no reflection (`"123456789"` gives `F4`).
* CRC-16/MODBUS: reflected poly `A001`, init `FFFF` (`"123456789"` gives `4B37`).
* CRC-32: zlib (reflected `EDB88320`, init and xorout `FFFFFFFF`; `"123456789"` gives `CBF43926`).
* Minimum frame: 11 bytes on WHOOP 4, 13 on WHOOP 5 (NOOP's empirical floor).
* WHOOP 5 `CLIENT_HELLO` is `AA 01 08 00 00 01 E6 71 23 01 91 01 36 3E 5C 8D`, i.e. command 145
  revision 1 with sequence 1. `HostCommand::GetHello.encode(Whoop5, 1)` reproduces it exactly.
* Validation order (matches NOOP's oracle): SOF, minimum size, declared = supplied length, header
  check (+ format byte), CRC-32. NOOP lets WHOOP 5 frames through whose `declared - 4` is not a
  multiple of 4; we follow that (strap-emitted frames are always padded).
* WHOOP 5 padding is indistinguishable from payload, so decoded bodies can carry up to three
  trailing zero bytes.

Packet types: 35 command, 36 command response (38 alias), 40 realtime HR, 43 raw sensor, 47
historical, 48 event, 49 history metadata (56 alias), 50 console log, 51/52 IMU streams. 43, 50, 51,
52 are passed through as `Packet::Unknown`.

## 3. Packet layouts

### 3.1 Command (35) and response (36)

Command inner: `[35][seq][cmd][body…]`. Response inner: `[36][strap seq][cmd][origin seq][result][body…]`.
Result: 0 failure, 1 success, 2 pending, 3 unsupported, other values preserved. Correlate on
`(cmd, origin seq)`; the strap's own sequence is not the echo. One request can produce a pending then
a final response, or duplicate finals.

| Command | W4 no. / body | W5 no. / body |
|---|---|---|
| Toggle realtime HR | 3 / `[on]` | 3 / `[on]` |
| Battery | 26 / `[0]` | 26 / `[0]` |
| Get clock | 11 / `[0]` | 147 / `[1]` |
| Set clock | 10 / `unix u32` + 4 zero bytes | 146 / `[1] unix u32 ticks u16` (1/32768 s) |
| Data range | 34 / `[0]` | 34 / `[0]` |
| Start history | 22 / `[0]` | 22 / `[0]` |
| History ack | 23 / `[1]` + 8-byte token | 23 / `[1]` + 8-byte token |
| Abort history | 20 / `[0]` | 20 / `[0]` |
| Hello | 35 / `[0]` | 145 / `[1]` |
| Version info | 7 / `[0]` | none |
| Extended battery | 98 / `[0]` | none |

Deliberately **not** exposed: reboot (29), power cycle (32), force trim (25), read pointer (33),
firmware commands. `HostCommand::Raw` can still send any number if the app chooses.

Response bodies decoded: battery (W4 `u16/10` percent, W5 `u32` whole percent), clock (W4 `u32`
seconds; W5 `[rev][u32][u16 ticks]`), data range, W4 version info, W4 extended battery (`u16` mV at
body offset 5).

### 3.2 Data range body (both families)

`[rev=1]` then 16 `u32`: boundary A, read page, write page, trim page, wrap, capacity (131072 in all
captures), record estimate (about 15 per page, not exact), derived backlog, then four
`(seconds, companion)` pairs for the boundary-A page, trim-side page, read-side page and last-written
page. Values are page slots, not timestamps. Seconds of 1262044800 (2010-01-01) or `FFFFFFFF` are
sentinels. A real WHOOP 4 capture carries 67 body bytes (two extra); WHOOP 5 gets its two from
padding. Verified on one real frame per family.

### 3.3 Realtime heart rate (40)

| inner | field |
|---:|---|
| 2 | `u32` device seconds |
| 6 | `u16` sub-second word (unit unverified) |
| 8 | `u8` bpm |
| 9 | `u8` R-R count |
| 10 | `u16` x up to 5, R-R (zero = placeholder) |

Inner length is 20 in real frames. WHOOP 4 R-R are milliseconds. WHOOP 5 words are treated as
milliseconds by default (see U3). Verified: one real WHOOP 5 frame (98 bpm, 603/587) and the
WHOOP 4 sample frame.

### 3.4 Events (48)

`[48][seq][event no][0][u32 time][payload…]`. `BATTERY_LEVEL` (3) payload: `u16` tenths of a
percent at inner 13, `u16` mV at 17, charging bit 0 at 22; same on both families. Verified on a real
WHOOP 5 battery event (49.9 %, 3833 mV) and a synthetic WHOOP 4 one. The name table (`event_number::name`)
is NOOP's.

### 3.4a History metadata (49 / 56)

`[49][seq][meta type][payload…]`, type 1 start, 2 end, 3 complete.

`END` payload (identical on both families, from `inner[3]`): `unix u32`, `subsec u16`, `reserved u32`,
`ack token[8]`. The token's first word is the trim cursor and the second the write-wrap state.
`START`: WHOOP 4 sends none; WHOOP 5 sends a longer payload starting with a Unix time (kept opaque).
Verified: real WHOOP 5 END (trim 112193, wrap word 16, matches that strap's data-range wrap of 16);
WHOOP 4 START/END/COMPLETE are NOOP fixtures whose END carries only a 4-byte token (decoder accepts
4..8 bytes).

### 3.5 Historical records (47)

WHOOP 4 v24 (and v12), inner length 96: counter `u32`@3, Unix `u32`@7, sub-second `u16`@11, bpm@17,
R-R count@18, R-R `u16`@19.. (ms, 5 max), gravity `f32` x3 @36/40/44, second gravity @52/56/60, raw
red/IR `u16`@64/66 (not SpO2 %), skin temperature raw ADC `u16`@68. Verified on one real 41.17.6.0
frame (109 bpm, R-R 555/564, raw temp 861).

WHOOP 4 v25 (84-byte frame): counter@3, Unix@7, three `i16` movement words @69/71/73 read as
gravity/16384. No heart rate. Verified on three real frames (|g| about 0.94-0.99 per NOOP).

WHOOP 4 v5/v7/v9: counter, Unix@7, bpm@17, R-R@18/19. Layout from NOOP's schema only; **no real
frame tested**.

WHOOP 5 R18 (inner 112, layout byte 18, marker 0x80): record index `u32`@3, Unix `u32`@7, bpm@14,
R-R count@15 (4 max), R-R `u16`@16, dynamic acceleration `f32`@33, gravity `f32` x3 @37/41/45, step
counter `u16`@49, activity@55, skin temperature `u16`@65 in centidegrees (30.57 C worn, 22.47 C off
wrist in captures). Verified on five real frames from two straps.

Everything else (WHOOP 4 other versions, WHOOP 5 R16/R17 ECG, R20 optical, R21 IMU, R22, R26) is
`HistoricalPacket::Unmapped`: archive the frame, do not guess.

`HistorySample` is the family-neutral view. Timestamps are whole seconds (`unix_ms = secs * 1000`).
WHOOP 4 skin temperature in degrees is a **provisional** linear map (33 C at raw 826, 0.05 C per
count) good only for deviation from a personal baseline; the raw register is always kept. SpO2 is
exposed only as raw red/IR scalars (WHOOP 4); NOOP documents no percentage conversion. Steps (W5
only) are a cumulative counter, not the app's daily step count.

## 4. History sync sequence

See `protocol/sync.rs` for the same diagram as code docs.

1. (WHOOP 5) write `CLIENT_HELLO`. Subscribe to every notify characteristic. (WHOOP 4) send a
   confirmed `GET_BATTERY_LEVEL`; once that write is acknowledged commands work. Run the connect
   handshake once per connection (re-running it mid-offload was observed to stop type-47 streaming).
2. Optional: `GET_HELLO`, `GET_CLOCK`, `SET_CLOCK` (then read the clock back: on some WHOOP 4 units one
   of the two set-clock forms latches and the other does not).
3. `GET_DATA_RANGE` (required first on WHOOP 5, optional on WHOOP 4). Expect pending (W5) then success.
4. `SEND_HISTORICAL_DATA` `[0]`. The response is only acceptance.
5. Strap sends metadata START, then type-47 records, then metadata END (about every 50 records).
6. Host decodes the chunk, **persists records, raw frames and the cursor durably, then** sends
   `HISTORICAL_DATA_RESULT` `[1] + ack token` with the token copied verbatim from END. An END with
   no records is still acknowledged.
7. Repeat 5-6 until metadata COMPLETE. COMPLETE does not commit an open chunk.
8. Afterwards compare with a fresh `GET_DATA_RANGE`. Unacknowledged data replays after reconnect, so
   store idempotently by timestamp.
9. Abort with command 20 (stop, not trim). Guard with an idle watchdog (NOOP: 60 s without
   type 47/48/49/50 frames) and resume next time.

`HistorySync::on_packet` yields `SyncEvent::Chunk(PendingChunk)`; `PendingChunk::ack(self)` consumes
the chunk and returns the ack command, so the ack cannot be built without the chunk in hand.

## 5. Unknowns and unverified items

* **U1** Negotiated ATT MTU / notification size is undocumented. `assumed_att_mtu()` is the BLE
  minimum (23). The reassembler does not depend on it.
* **U2** WHOOP 5 role of `fd4b0003/4/5/7` is by analogy with WHOOP 4; `0007` unknown.
* **U3** WHOOP 5 R-R unit. NOOP's written 50.42.1.0 contract says 1/1024 s ticks; its shipping
  decoder and oracle treat the word as milliseconds. We default to milliseconds
  (`CodecOptions::whoop5_rr`); verify against a strap with a chest-strap reference before trusting HRV.
* **U4** WHOOP 4 device-clock semantics. Historical/event times looked like real Unix time in captures,
  but NOOP's own type-40 fixtures use device seconds that need a clock reference. Set the clock and
  read it back; derive an offset from `GET_CLOCK` if needed.
* **U5** WHOOP 4 skin temperature calibration; per-device offset (second strap needed a different
  anchor). Treat as relative only.
* **U6** WHOOP 4 battery body beyond the first `u16`; encoder appends four zero bytes copied from a
  NOOP fixture whose provenance NOOP itself questions (its result byte reads 0 despite success).
* **U7** WHOOP 4 `GET_DATA_RANGE` body is 67 bytes in the one capture (65 documented); the last two
  bytes are unmapped.
* **U8** Whether a strap retransmits an unacknowledged END byte-for-byte, replay after disconnect, ring
  wrap semantics, and maximum chunk size are unresolved (NOOP lists them open).
* **U9** WHOOP 4 v5/v7/v9/v12 layouts and the `skin_contact` byte are NOOP schema claims, not
  re-verified (byte 55 of the real v24 frame is just the top byte of a float 4.0).
* **U10** WHOOP 5 METADATA `START` payload (36+ bytes) and the sub-second words are opaque.
* **U11** WHOOP 4 `SET_CLOCK` fractional part and WHOOP 5 clock-set behavior with an unset RTC.
* **U12** Hello bodies (WHOOP 4 serial plus key material, WHOOP 5 identity blocks) are not decoded on
  purpose; they contain identifiers and must not be logged.
* **U13** WHOOP 5 realtime frame tail (6 bytes after R-R words in the real frame) is unmapped; the
  encoder zeros it.
* **U14** Type 43 raw data (WHOOP 4 1917/1921-byte layouts, WHOOP 5 R16/R17 ECG), IMU 51/52 and
  console logs are not decoded. Command 63 (WHOOP 4) stops the roughly 2/s type-43 flood.
* **U15** Command 3 (toggle realtime HR) is outside NOOP's documented 41.17.6.0 command set though
  NOOP sends it; the standard `2A37` service is the dependable live-HR source.

## 6. Test coverage

`cargo test -p unstrapped-core protocol`: CRC check values; 45 frame-integrity cases (both
families: valid frames, corrupt header and payload CRCs, truncation, trailing bytes, undersized
frames, missing start byte); `CLIENT_HELLO` reproduced byte for byte; WHOOP 5 command frames checked
against an envelope assembled by hand from the documented layout; golden frames decoded (WHOOP 4
realtime/v24/v25/data range/metadata/events, WHOOP 5 realtime/R18/events/battery/data range/END) and
re-encoded byte for byte; every packet kind
and every host command round-tripped encode, fragment (MTU 23/185/517), reassemble, decode on both
families; per-byte corruption rejection; reassembler garbage, false-SOF, corrupt-CRC and
multi-frame cases; sync persist-before-ack flow.
