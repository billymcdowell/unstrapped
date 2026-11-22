# WHOOP strap emulator

`crates/unstrapped-emulator` is a stand-in WHOOP 4.0 / 5.0 strap. It speaks the real byte-level protocol
from `unstrapped_core::protocol` (CRCs, fragmentation, commands, history offload) and generates
reproducible synthetic physiology, so the app can be developed and tested without hardware.

## Running it

```
cargo run -p unstrapped-emulator -- --family whoop4 --time-scale 60
cargo run -p unstrapped-emulator -- --family whoop5 --seed 9 --fast-history --start 2026-03-01T08:00:00Z
```

Options (all optional):

| flag | meaning |
|---|---|
| `--addr HOST:PORT` | listen address, default `127.0.0.1:47800` |
| `--family whoop4\|whoop5` | strap generation, default `whoop4` |
| `--seed N` | RNG seed. Same seed + same clock + same timezone gives identical bytes |
| `--history-days N` | history held on the strap, 1-14 (default 14); older data is overwritten |
| `--time-scale X` | emulated seconds per real second (60 = a minute per second). Live HR still arrives once per real second, sampled every `X` emulated seconds |
| `--start MS\|RFC3339` | emulated clock at boot (default: now) |
| `--fast-history` | stream history as fast as the host acks (default ~150 records/s, BLE-like) |
| `--record-interval S` | seconds between stored records (default 1, as on a real strap) |
| `--tz-offset MIN` | local timezone, minutes east of UTC; bed/wake times are local. Default: system offset |
| `--name NAME` | advertised name |
| `--age --sex --weight --rhr --hrv --max-hr --bedtime HH:MM --wake HH:MM` | subject profile |

Dev controls can be sent with the `ctl` subcommand (or `HostMsg::Control` / `EmulatorClient::control`):

```
cargo run -p unstrapped-emulator -- ctl take_off            # strap off the wrist (history gap + wrist event)
cargo run -p unstrapped-emulator -- ctl put_on
cargo run -p unstrapped-emulator -- ctl advance_minutes 600 # jump the emulated clock forward
cargo run -p unstrapped-emulator -- ctl set_battery 15
cargo run -p unstrapped-emulator -- ctl charge_on           # / charge_off (charging also stops recording)
cargo run -p unstrapped-emulator -- ctl out_of_range        # drops the host; advertises in_range:false
cargo run -p unstrapped-emulator -- ctl in_range
```

Embedding: `unstrapped_emulator::server::serve(addr, config)` (defaults) or `serve_with(addr, config,
ServerOptions)` / `serve_on(listener, ..)`. `ServerOptions` carries `fast_history`, `tick_ms` and a
`StrapOptions` (record interval, ATT MTU, chunk size, throttle, timezone). `EmulatedStrap` itself
is synchronous and deterministic (`EmulatedStrap::with_options`) and is what the tests drive.

## Wire format

Newline-delimited JSON over TCP, see `src/wire.rs`. On accept the emulator sends `Advertisement`.
One host at a time: a second connection gets an `Error` line and is closed. While out of range, a
connection receives `Advertisement { in_range: false }` and only accepts `Control` messages
(`EmulatorClient::connect` errors; `EmulatorClient::connect_any` still connects so a tool can send
`in_range`). `Disconnect` leaves the TCP session open so the host can `Connect` again. The
characteristic list sent in `Connected` is the family's custom characteristics plus the standard
Heart Rate Measurement (`2a37`) and Battery Level (`2a19`) notify characteristics.

## What the strap does

Both families (WHOOP 4: `61080002..5`, WHOOP 5: `fd4b0002..5, 0007`), notifications fragmented with
`fragment(frame, mtu)` (default MTU 23) and only delivered for subscribed characteristics:

* command responses on the command-response characteristic, events on the event characteristic,
  realtime HR / history records / history metadata on the data characteristic;
* commands: toggle realtime HR (also emits event 33/34), battery (W4 `u16/10 %`, W5 `u32 %`), W4
  extended battery, get/set clock (set shifts the strap clock only), get data range (W5 answers
  Pending first), send historical data, historical data result (ack), abort, hello, W4 version info;
  anything else answers `Unsupported`;
* realtime: about one `RealtimeHr` packet per second with R-R intervals, plus `0x2A37` packets
  (contact flag, R-R in 1/1024 s) while realtime is on;
* events: wrist on/off (9/10), battery level (3) on every whole-percent change, charging on/off
  (7/8); battery also on `2a19`. Battery drains ~22 %/day, charges ~1.3 %/min.

History offload follows `docs/PROTOCOL_NOTES.md` section 4: metadata START, records, metadata END
with an 8-byte token (`[last record index u32][16 u32]`), then the strap waits for
`HISTORICAL_DATA_RESULT` with that token before sending the next chunk (50 records per chunk), and
finally COMPLETE. The strap only trims on an ack; a new transfer (after an abort, a disconnect or
another `SendHistoricalData`) restarts from the first unacked record, so unacknowledged chunks
replay. Records are stored implicitly: the physiology is regenerated per timestamp, the strap keeps
only the trim cursor and the window `[now - history_days, now]`, so memory use is constant and data
older than `history_days` is silently gone ("overwritten"). Record index = timestamp / interval.
WHOOP 4 records are layout v24 (HR, R-R, gravity, raw SpO2 red/IR, raw skin temp), WHOOP 5 records
are R18 (HR, R-R, dynamic accel, gravity, step counter, skin temp in centidegrees).

## Synthetic data

Everything is a pure function of (seed, profile, timestamp) (`physio::Physiology`, SplitMix64
keyed by seed + time), so the same second always yields the same record.

* Nights: bedtime/wake jitter, sleep-onset latency 3-40 min, ~90 min cycles (deep early, REM late),
  2-4 brief awakenings, HR falling after onset with a slow rise toward morning, stage-dependent HR /
  HRV / breathing regularity.
* Days: sitting, walking bouts, occasional brisk walks, a run on ~40 % of days (30-55 min, peak
  72-84 % max HR), warm-up / cool-down ramps and post-exercise recovery. Every 14th day (by local
  day index) is high strain (80-100 min hard run, lower HRV the next night) and one is "rough"
  (HRV x0.68, RHR +2.5, resp rate +1.6); random extras occur too. Night HRV varies ~20 % night to night.
* R-R intervals are generated per minute with respiratory sinus arrhythmia (13-16 breaths/min asleep)
  and noise sized so the series RMSSD equals the planned HRV; block boundaries join seamlessly.
* Skin temperature has a circadian curve, a ~1.1 C rise during sleep and per-night deviations.
* Motion is encoded as gravity vectors; the change between consecutive 1 Hz samples is about 0.004 g
  asleep, 0.02 g seated, 0.15 g walking and 0.4-0.5 g running, which is what the metrics engine expects
  once the host computes `magnitude_g` as the L2 norm of the change.
* About a third of days contain a 20-50 min strap-off period (10:00-15:00 local): no records, plus
  wrist-off/on events when connected. `take_off` / `charge_on` also leave gaps.
* WHOOP 4 raw SpO2 uses an emulator convention `SpO2 = 110 - 25 * red / ir`
  (`physio::spo2_from_raw`). It is NOT a WHOOP calibration (none is documented).
* Step counter (W5 only, u16 wrapping) is cumulative from the strap's "boot day"; the host adds the
  wrap-safe differences.

`cargo test -p unstrapped-emulator` also feeds six days of decoded samples into
`unstrapped_core::metrics::compute_day` (sleep within minutes of the planned bed/wake, HRV/resp rate near
the plan, workouts detected, strain 0-21, recovery scored once 4 prior days exist).

## Limitations

* Behaviour that is unverified in `PROTOCOL_NOTES.md` (U1-U15) is emulated by assumption: ATT MTU 23,
  W5 R-R in milliseconds, W4 clock = Unix time, END tokens of 8 bytes, a strap that waits for each ack.
* Not modelled: type-43 raw sensor floods, IMU/console packets, ECG/optical W5 record layouts, haptics,
  alarms, firmware commands, bonding, reboot/trim commands (answered `Unsupported`).
* WHOOP 4 v24 records carry no step counter, so the W4 data has no steps.
* The strap clock offset set by `SET_CLOCK` only affects `GET_CLOCK`, realtime and event stamps;
  stored records always use true emulated time.
* Timezone: the default is the machine's current UTC offset (fixed for the process, no DST changes),
  so the bed/wake times land at the configured local clock time. Pass `--tz-offset 0` for
  machine-independent output.
* `advance_minutes` skips time but does not emit the wrist/battery events that would have occurred.
* Realtime HR is paced by the wall clock: one reading per real second whatever `--time-scale` is
  (with `--time-scale 60` consecutive readings are a minute apart in the strap's day). History
  throughput is throttled by wall-clock ticks (`tick_ms`, default 50 ms).
