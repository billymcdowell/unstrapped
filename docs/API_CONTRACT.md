# Strap: front end <-> Rust API contract

Source of truth for types: [`src/api/types.ts`](../src/api/types.ts). Typed client: [`src/api/client.ts`](../src/api/client.ts). Mock backend with the same shapes: [`src/api/mock.ts`](../src/api/mock.ts) (used automatically when `window.__TAURI_INTERNALS__` is absent).

## Rules for the Rust side

- All structs/enums: `#[derive(Serialize, Deserialize)] #[serde(rename_all = "camelCase")]`. JSON keys equal the TS field names.
- JS call args are camelCase; Tauri maps them to **snake_case Rust parameters**. The "Rust params" column gives the exact names (JS key in brackets where it differs).
- `Day` = `String` "YYYY-MM-DD" (local date of the day the main sleep ended). `Ms` = `i64` Unix milliseconds. Ranges/enums are lowercase/camelCase strings as written in the TS unions.
- Commands return `Result<T, String>`; the `Err` string is shown to the user as the error message.
- `TimeSeries` is columnar: `{ "ts": [ms...], "v": [number|null...] }`, equal lengths, `null` = no reading (breaks the line).
- **`Result<T>` (state wrapper)** used wherever data may be unavailable. Never send zeros for missing data:
  ```rust
  #[derive(Serialize)] #[serde(tag = "state", rename_all = "camelCase")]
  enum Avail<T> { Ok { data: T }, Calibrating { nights_remaining: u32, message: Option<String> }, NoData { reason: String, gaps: Vec<Gap> } }
  ```
  JSON: `{"state":"ok","data":{...}}`, `{"state":"calibrating","nightsRemaining":2}`, `{"state":"noData","reason":"...","gaps":[{"startMs":..,"endMs":..,"reason":"Strap was off"}]}`. (TS name is `Result<T>`.)
- After any change to stored/derived data (sync finished, activity added/renamed, rebuild, journal edit that changes insights) emit `data://updated` with the affected days; open screens refetch.

## Commands

| Command (Rust name) | Rust params | Returns (TS type) |
|---|---|---|
| `get_home` | `day: String` | `HomeData` |
| `get_sleep` | `day: String` | `Result<SleepData>` |
| `get_recovery` | `day: String` | `Result<RecoveryData>` |
| `get_strain` | `day: String` | `Result<StrainData>` |
| `get_activity` | `id: String` | `ActivityDetail` |
| `add_activity` | `kind: String`, `start_ms: i64` [`startMs`], `end_ms: i64` [`endMs`] | `ActivitySummary` (manual=true) |
| `rename_activity` | `id: String`, `kind: String` | `ActivitySummary` |
| `get_health` | `day: String` | `HealthData` |
| `get_metric_detail` | `metric: String` (`respRate`\|`spo2`\|`restingHr`\|`hrv`\|`skinTemp`), `day: String` | `Result<MetricDetail>` |
| `get_stress` | `day: String` | `Result<StressData>` |
| `get_trend` | `metric: String` (`TrendMetric`), `range: String` (`1W`\|`1M`\|`6M`\|`1Y`) | `Result<TrendData>` |
| `get_journal` | `day: String` | `JournalDay` |
| `get_journal_library` | none | `Vec<JournalLibraryItem>` |
| `set_journal_behaviors` | `ids: Vec<String>` | `()` |
| `set_journal_answer` | `day: String`, `behavior: String`, `answer: Option<bool>` (null clears) | `()` |
| `get_insights` | none | `Vec<Insight>` (90-day window; `state:"notEnough"` unless >= 5 yes and >= 5 no) |
| `list_assessments` | none | `Vec<AssessmentListItem>` (newest first) |
| `get_assessment` | `id: String` | `Assessment` |
| `get_healthspan` | none | `Result<Healthspan>` |
| `get_raw_day` | `day: String` | `RawDay` (per-minute HR for the 24 h + spans) |
| `get_raw_night` | `day: String` | `Result<RawNight>` (HR + R-R intervals during the main sleep) |
| `get_raw_trends` | `range: String` | `RawTrends` (resting HR + HRV per day) |
| `export_csv` | `kind: String` (`heart_rate`\|`rr_intervals`\|`daily_summary`\|`sleeps`\|`activities`\|`stress`\|`raw_frames`), `from: String`, `to: String` (inclusive days) | `String` absolute path of the saved file (e.g. in Downloads) |
| `get_strap_status` | none | `StrapStatus` |
| `connect_strap` | `transport: String` (`ble`\|`emulator`) | `()` (returns when connect attempt finished; state changes also via `strap://status`) |
| `disconnect_strap` | none | `()` |
| `sync_now` | none | `()` (resolves when done; progress via `sync://progress`; emits `data://updated`) |
| `rebuild_decoded` | none | `()` (emit `sync://progress` phase `processing` while running, then `data://updated`) |
| `get_settings` | none | `Settings` |
| `set_settings` | `settings: Settings` | `Settings` (the stored value) |
| `emulator_control` | `action: String` (`out_of_range`\|`in_range`\|`take_off`\|`put_on`\|`advance_minutes`), `value: Option<i64>` (minutes for `advance_minutes`, else null) | `()` |

Notes:
- `Settings` = `{ profile:{age,sex:"male"|"female"|"other",weightKg,heightCm}, transport:"ble"|"emulator", emulatorAddress:"127.0.0.1:47800", autoSync, launchAtLogin }`. The front end calls `set_settings` before `connect_strap`, so `emulatorAddress` must be read from stored settings by connect.
- `get_home` pieces are independently `Result<>`-wrapped, so Home can show Recovery calibrating while Strain is fine.
- `HomeData.strain.targetLo/targetHi` and `StrainData.targetLo/targetHi` are `null` when no target exists (no recovery yet).
- `SleepData.contributors` always has the four entries `hoursVsNeeded`, `consistency`, `efficiency`, `sleepStress`; `pct` is 0-100 or `null` when that contributor cannot be computed (the UI shows "—", never a fake 0). `StrainData.stats.calories` / `avgHr` are likewise `number | null`. `consistency` has the last 14 nights ending at `day`. Planner options: `peak` 100, `perform` 85, `getBy` 75.
- `StrainData.zoneBounds` / `ActivityDetail.zoneBounds` = lower bpm of zones 2..5. `zoneMin` = minutes in zones 1..5. `hr` per-minute for the whole day (up to now for today).
- `RecoveryData.contributors` has four entries: `hrv`, `restingHr`, `respRate`, `sleepPerformance`; `impactPts` is signed points added to the score; `direction` accounts for lower-is-better metrics.
- `TrendData.granularity` is `daily` for 1W/1M and `weekly` (average per week, `day` = first day of week) for 6M/1Y. `band` is set only for `recovery`.
- Activity ids are opaque strings, stable across syncs.

## Events (Rust -> JS, `app.emit`)

| Event | Payload (TS type) | When |
|---|---|---|
| `strap://status` | `StrapStatus` | any connection/battery/wrist/last-sync change |
| `strap://raw` | `RawFrameEvent` `{ receivedAt: Ms, characteristic: String, hex: String }` | every BLE notification (rate-limit if needed; UI keeps last 400) |
| `strap://hr` | `HrEvent` `{ ts: Ms, bpm: u16 }` | each decoded live HR sample |
| `sync://progress` | `SyncProgress` `{ phase, fraction: 0..1, label }` e.g. label `"Downloading 23 h of data… 64%"`; phases `idle`\|`scanning`\|`connecting`\|`requesting`\|`downloading`\|`processing`\|`done`\|`error`. Emit `idle` at the end to clear the bar. | during sync/rebuild |
| `data://updated` | `DataUpdatedEvent` `{ days: Vec<String> }` | after sync, activity changes, rebuild |

`StrapStatus.state`: `disconnected`\|`scanning`\|`connecting`\|`connected`\|`syncing`\|`error`.

## Tauri setup done by the front-end agent

- `src-tauri/tauri.conf.json`: window `main`, title "Strap", transparent, `titleBarStyle: Overlay`, hidden title, `macOSPrivateApi: true`.
- `tauri-plugin-liquid-glass` 0.1.6 registered in `lib.rs`; capability `liquid-glass:default`.
- Backend: register new commands in `invoke_handler![...]`; add `core:event:default` is already in `core:default`.

## Additions by the screens agent (additive)

- `Assessment.zonesByActivity[]` gained optional `avgStrain?: number | null` (mean activity strain for that type in the period) and `sessions?: number`. The monthly report shows them when present.
- Semantics the screens assume: `Insight.recoveryDelta` and `sleepDelta` are percentage points (yes minus no), `hrvDelta` is percent change in mean next-day HRV. All three are shown with a `%` suffix.
- `TrendData.summary.best/worst` should respect direction (lower is better for `restingHr`, `respRate`, `sleepDebt`, `stress`); the UI labels them "Best"/"Worst" as given.
