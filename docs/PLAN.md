# Plan: a Mac app that syncs your WHOOP strap directly

_Tauri 2 + Rust + the Liquid Glass plugin · laptop-first, no phone app · written 19 Nov 2026_

## The approach in one paragraph

Your strap stores about 14 days of second-by-second data on its own. So instead of a phone app that is always connected, you build a **Mac app** that connects to the strap over Bluetooth once a day, downloads everything since the last sync, and keeps it in a local database. All the charts and scores (recovery, strain, HRV, sleep) are calculated on the Mac. The interface uses Apple's Liquid Glass look via `tauri-plugin-liquid-glass`. There's no sideloading, no iOS background limits and no server needed. A phone app or a backup server can be added later; neither changes this design.

```
WHOOP strap ──Bluetooth LE, once a day──▶ Mac app (Tauri)
                                          ├─ Rust: Bluetooth (btleplug)
                                          ├─ Rust: whoop-core (decode packets)
                                          ├─ SQLite: raw packets + decoded values
                                          ├─ Rust: metrics (recovery, strain, sleep)
                                          └─ Web UI with Liquid Glass window
```

**Assumptions:** you're on a Mac. Real Liquid Glass needs **macOS 26 (Tahoe) or later**; on older macOS the plugin falls back to the older frosted "vibrancy" effect, so the app still works.

---

## Before you start

| What                     | Why                                  | How                                                                                 |
| ------------------------ | ------------------------------------ | ----------------------------------------------------------------------------------- |
| Xcode command line tools | Compiler and linker for macOS        | `xcode-select --install` (the full Xcode app isn't needed for a Mac-only Tauri app) |
| Rust                     | The app's core                       | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh`                   |
| Node.js (LTS)            | The web front end and the Tauri CLI  | From nodejs.org or `brew install node`                                              |
| Your WHOOP strap         | The data source                      | Worn or charging, so it reports real readings                                       |
| NOOP source code         | Reference for the Bluetooth protocol | `git clone https://github.com/ryanbr/noop`                                          |

**About NOOP and its license.** NOOP's Mac app already does what you're building: it talks to the strap directly from a Mac. Use it as your reference. Its code is under the PolyForm Noncommercial license (personal, non-commercial use is fine). Its license notice also says the protocol facts — Bluetooth IDs, packet layouts, checksum settings, command numbers — aren't copyrightable and can be reused freely. If you ever want to sell your app, write your own code from those facts rather than translating NOOP's code.

**About the official WHOOP app.** NOOP's guide says the strap must be out of range of the official WHOOP app when it first pairs with NOOP. Also, the strap probably only keeps history until a device confirms it has received it. If the official app keeps syncing from your phone, it may take the history before your Mac does. Plan to use one or the other, not both.

---

## Phase 0 — Understand the protocol (reading only)

1. In the NOOP repo, read `docs/PROTOCOL.md`, then `docs/PROTOCOL_TRANSPORT.md`, then the file for your strap: `docs/PROTOCOL_WHOOP4.md` or `docs/PROTOCOL_WHOOP5.md`.
2. Work out which strap you have. Each generation advertises a different Bluetooth service:
   - WHOOP 4.0: `61080001-8d6d-82b8-614a-1c8cb0f8dcc6`
   - WHOOP 5.0 / MG: `fd4b0001-cce1-4033-93ce-002d5875f58a`
3. Find the characteristic IDs (the channels you write commands to and receive data from) in `Packages/WhoopProtocol/Sources/WhoopProtocol/DeviceFamily.swift`.
4. Understand the checks each packet goes through: a header checksum (CRC8 on 4.0, CRC16-Modbus on 5.0), a declared length, and a CRC32 at the end. One packet can arrive split across several Bluetooth notifications and has to be put back together.

**Done when:** you can sketch one packet's layout for your strap on paper.

---

## Phase 1 — Create the Tauri app

1. Scaffold it:
   ```bash
   npm create tauri-app@latest whoop-mac
   # pick: TypeScript, and a front end you like (React, Svelte or vanilla)
   cd whoop-mac
   npm install
   npm run tauri dev
   ```
2. Set up the folders so the protocol code lives in its own crate. You can test it without the app, and reuse it in a phone app later:
   ```
   whoop-mac/
   ├─ src/                  # web UI
   ├─ src-tauri/            # the Tauri app (commands, Bluetooth, database)
   └─ crates/
      └─ whoop-core/        # framing, CRCs, decoding, metrics (pure Rust, no Tauri)
   ```
   Make the project root a Cargo workspace that includes `src-tauri` and `crates/whoop-core`.

**Done when:** the starter window opens on your Mac.

---

## Phase 2 — Add the Liquid Glass window

This is quick and gives the app its look before the hard Bluetooth work starts.

1. Install the plugin:
   ```bash
   cd src-tauri && cargo add tauri-plugin-liquid-glass && cd ..
   npm install tauri-plugin-liquid-glass-api
   ```
2. Register it in `src-tauri/src/lib.rs`:
   ```rust
   tauri::Builder::default()
       .plugin(tauri_plugin_liquid_glass::init())
       // ...other plugins
       .run(tauri::generate_context!())
       .expect("error while running tauri application");
   ```
3. Allow the front end to use it in `src-tauri/capabilities/default.json`:
   ```json
   {
     "identifier": "default",
     "windows": ["main"],
     "permissions": ["core:default", "liquid-glass:default"]
   }
   ```
4. Make the window transparent and allow the private macOS API in `src-tauri/tauri.conf.json`:
   ```json
   {
     "app": {
       "macOSPrivateApi": true,
       "windows": [
         {
           "label": "main",
           "title": "Strap",
           "transparent": true,
           "titleBarStyle": "Overlay",
           "hiddenTitle": true
         }
       ]
     }
   }
   ```
5. Make the page itself see-through, otherwise the glass is hidden behind it:
   ```css
   html,
   body {
     background: transparent;
   }
   ```
6. Turn the effect on when the app starts:

   ```ts
   import {
     isGlassSupported,
     setLiquidGlassEffect,
     GlassMaterialVariant,
   } from "tauri-plugin-liquid-glass-api";

   // On macOS 26+ this is real Liquid Glass; on older macOS the plugin falls back to vibrancy.
   await setLiquidGlassEffect({
     cornerRadius: 16,
     variant: GlassMaterialVariant.Regular, // try Sidebar or Clear too
   });
   const realGlass = await isGlassSupported(); // e.g. to adjust text contrast
   ```

**Design tips:** keep the glass to the window background and a few panels, not every card. Put charts on slightly solid panels so lines stay readable over whatever is behind the window. Test in both light and dark mode.

**Know the trade-off:** the plugin uses Apple's undocumented `NSGlassEffectView`. That's fine for an app you run yourself, but a macOS update could break it, and it would likely block a Mac App Store release. If it breaks, call `setLiquidGlassEffect({ enabled: false })` and the app keeps working without the effect.

**Done when:** your app window shows the glass effect.

---

## Phase 3 — Connect to the strap over Bluetooth

1. Add the Rust Bluetooth library to `src-tauri`:
   ```bash
   cd src-tauri && cargo add btleplug tokio --features tokio/full && cargo add uuid futures && cd ..
   ```
   (Alternative: `tauri-plugin-blec`, which wraps btleplug with a ready-made JavaScript API. Doing it directly in Rust keeps all the strap logic in one place.)
2. Tell macOS why the app needs Bluetooth. Create `src-tauri/Info.plist`:
   ```xml
   <?xml version="1.0" encoding="UTF-8"?>
   <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
   <plist version="1.0">
   <dict>
     <key>NSBluetoothAlwaysUsageDescription</key>
     <string>Connects to your WHOOP strap to download your data.</string>
   </dict>
   </plist>
   ```
3. In Rust, write a `strap` module that:
   - scans for the WHOOP service ID from Phase 0
   - connects to the strap it finds and remembers its ID, so next time it connects straight away
   - discovers its characteristics and subscribes to the ones that send notifications
   - forwards every notification's raw bytes to the rest of the app, with a timestamp and the characteristic ID
4. Expose it to the UI with Tauri commands (`connect_strap`, `disconnect_strap`) and send the bytes to the UI with Tauri events so you can watch them.

**Gotcha in development:** under `npm run tauri dev` the app isn't a bundled `.app` yet, so macOS asks for Bluetooth permission for **your terminal app** instead. Allow it in System Settings → Privacy & Security → Bluetooth.

**Done when:** hex bytes scroll in the app while you wear the strap.

---

## Phase 4 — Decode packets: live heart rate

1. In `crates/whoop-core`, write:
   - a **reassembler** that joins notification fragments into complete packets using the declared length
   - **checksums**: CRC8 (polynomial 0x07) or CRC16-Modbus for the header, depending on strap, and CRC32 for the body. Reject any packet that fails.
   - a **parser** that turns a valid packet into typed values. Start with the live heart-rate stream only.
2. Test before you plug it into the app. NOOP's `docs/protocol-examples` folder has sample packets. Write `cargo test` cases that feed them in and check the decoded values match what NOOP expects.
3. Wire it into the app: raw bytes go into `whoop-core`, and decoded heart rate goes to the UI.

**Done when:** the app shows your live heart rate and it looks right.

---

## Phase 5 — Store everything locally

1. Use SQLite via `rusqlite` (or `tauri-plugin-sql`). Put the database file in the app's data folder (Tauri's `app_data_dir()`, under `~/Library/Application Support/<your app id>/`).
2. Create two kinds of table:

   ```sql
   -- Every valid packet, exactly as received. Never edited or deleted.
   CREATE TABLE raw_frames (
     id            TEXT PRIMARY KEY,   -- random UUID
     received_at   INTEGER NOT NULL,   -- Unix ms, Mac clock
     strap_id      TEXT NOT NULL,
     channel       TEXT NOT NULL,      -- characteristic UUID
     kind          TEXT NOT NULL,      -- 'live' or 'history'
     bytes         BLOB NOT NULL
   );

   -- Decoded values, rebuildable from raw_frames at any time.
   CREATE TABLE heart_rate (
     ts_ms         INTEGER NOT NULL,   -- strap's timestamp
     bpm           INTEGER NOT NULL,
     frame_id      TEXT NOT NULL REFERENCES raw_frames(id),
     PRIMARY KEY (ts_ms)
   );
   -- add rr_intervals, temperature, motion, etc. as you decode them
   ```

3. Add a "rebuild decoded data" command that clears the decoded tables and re-decodes from `raw_frames`. When you fix a decoder bug, run it and all your history is corrected.

**Done when:** you can quit and relaunch the app and yesterday's readings are still there.

---

## Phase 6 — Download the strap's stored history

This is the heart of the once-a-day approach.

1. Follow NOOP's history notes (`PROTOCOL_TRANSPORT.md` and the file for your strap): ask the strap what range of history it holds, then request it in chunks.
2. **Save each chunk to `raw_frames` before acknowledging it to the strap.** NOOP's documentation is explicit about this. If you acknowledge first and the app then crashes, that data may be gone from the strap for good.
3. Keep a small `sync_state` table: last successful sync time, and how far into the strap's history you've got. An interrupted sync then resumes where it stopped.
4. Show progress in the UI ("Downloading 23 h of data… 64%").

**Done when:** you leave the Mac alone for a day, sync, and the whole day fills in with no gaps.

---

## Phase 7 — Make syncing automatic

The goal is that you never have to remember to sync.

1. **Menu bar app.** Use Tauri's tray icon support so the app lives in the menu bar and keeps running with its window closed. Show the last sync time in the menu.
2. **Start at login** with `tauri-plugin-autostart`.
3. **Auto-sync.** Every 15–30 minutes, scan briefly for the strap. If it's in range and the last sync was more than a few hours ago, sync quietly. If you're at your desk most days, this means daily syncs with no effort.
4. **Overdue warning.** With `tauri-plugin-notification`, notify you if there's been no sync for 7 days, and more urgently at 10. The strap only holds about 14 days, so missing that window means losing the oldest data.
5. **Sleep and App Nap.** macOS may slow down background apps. If syncs get unreliable, tell the system not to throttle the app while a sync is running.

**Done when:** a week goes by without you opening the app, and every day is in the database.

---

## Phase 8 — A first dashboard (raw data)

This is a simple data view to check that syncing works. The full health screens with scores come in Phase 9.

1. Pick a chart library for the web UI (for example uPlot for fast time series, or Recharts or ECharts).
2. Start with three views:
   - **Today:** heart rate over 24 h, with sleep shaded
   - **Night:** overnight heart rate and R-R intervals (beat-to-beat timing), the basis of HRV
   - **Trends:** resting heart rate and HRV across weeks
3. Add Tauri commands that run SQL queries in Rust and return only what a chart needs (for example, one point per minute), not millions of raw rows.
4. Add **Export to CSV** so you can analyse data in a spreadsheet or Python.
5. Put the glass on the window and sidebar; keep chart panels slightly solid for contrast.

**Done when:** you open the app in the morning and see last night's sleep and heart rate.

---

## Phase 9 — Health scores and screens that match the WHOOP app

This phase builds the screens you'll use every day. Each one shows **the same data, on the same scales, with the same kind of chart** as the WHOOP app, based on WHOOP's own descriptions of its app (as of November 2026; sources at the end).

**What "the same" can and can't mean:**

- **Same data, scales and chart types:** yes. Recovery and Sleep 0–100%, Strain 0–21, Stress 0–3, the same contributors on each screen, the same colour bands.
- **Same numbers:** no. WHOOP's formulas are private. You'll use published methods (via NOOP) aimed at the same questions, so your scores should move the same way as WHOOP's but won't match exactly.
- **Same look:** use your own visual design, name and icons rather than WHOOP's branding and logos. The information layout below is what matters.
- **Left out:** ECG and blood pressure (medical features that need special hardware, calibration and regulatory clearance), lab-result uploads, the AI coach and community teams.

### 9.1 — Raw measurements

Build these in `whoop-core` first, each as a tested function. Every screen is built from them.

| Measurement              | How                                                                             | Used on                         |
| ------------------------ | ------------------------------------------------------------------------------- | ------------------------------- |
| Heart rate               | Decoded directly, ~1 per second                                                 | Everywhere                      |
| R-R intervals            | Decoded directly (beat-to-beat timing)                                          | HRV, stress, sleep              |
| **HRV**                  | RMSSD in ms, from cleaned R-R intervals during sleep                            | Recovery, Health Monitor        |
| **Resting heart rate**   | Lowest steady stretch during sleep                                              | Recovery, Health Monitor        |
| **Respiratory rate**     | Breaths/min, from the breathing rhythm in R-R intervals during sleep            | Recovery, Sleep, Health Monitor |
| **Skin temperature**     | Strap's temperature sensor, shown as the difference from your normal            | Health Monitor                  |
| **Blood oxygen (SpO2)**  | Only if NOOP's protocol docs show your strap reports it; otherwise hide the row | Health Monitor                  |
| **Sleep and wake**       | Stillness + low heart rate; tidy up short blips                                 | Sleep                           |
| **Sleep stages**         | Awake / Light / Deep (SWS) / REM per 30 s, from HR, HRV, breathing and motion   | Sleep                           |
| **Disturbances**         | Count of wake events during the sleep                                           | Sleep                           |
| **Max heart rate**       | Highest observed, or 208 − 0.7 × age                                            | Strain, zones                   |
| **Heart-rate zones 1–5** | 50/60/70/80/90% of heart-rate reserve                                           | Strain, activities              |
| **Calories**             | Estimated from heart rate, age, sex and weight (which you enter once)           | Strain                          |
| **Steps**                | Only if your strap reports steps or you can count them from the motion data     | Strain                          |
| **Live stress**          | Current HR and HRV against a 14-day baseline, ignoring periods of movement      | Stress Monitor, Sleep           |

### 9.2 — Personal baselines

Every score compares you with your own normal.

- **HRV, resting HR, respiratory rate, skin temperature, SpO2:** a rolling baseline with recent nights weighted more (NOOP uses a ~14-night half-life) and a "normal range" around it. This drives Recovery and the Health Monitor.
- **Stress:** WHOOP uses a 14-day baseline of your HR and HRV.
- **Sleep need:** a baseline learned over your first ~28 days, like WHOOP.
- **Calibrating:** until there are enough nights (4 at minimum, 14 for solid), show "Calibrating" instead of a score.

### 9.3 — The scores, on WHOOP's scales

**Recovery (0–100%)** — "How ready is my body today?"

- WHOOP's inputs: HRV, resting heart rate, respiratory rate and sleep performance, plus SpO2 and skin temperature.
- Your formula: start from NOOP's documented weights (HRV 55%, resting HR 20%, sleep 15%, respiratory rate 5%, skin temp 5%). Compare each with your baseline, combine, and map to 0–100.
- Colour bands, same as WHOOP: **red 0–33, yellow 34–66, green 67–100.**

**Sleep Performance (0–100%)** — "How good was last night?"
WHOOP's current score has four contributors, and your screen shows the same four:

| Contributor       | What it measures                                       | Suggested starting weight (your choice) |
| ----------------- | ------------------------------------------------------ | --------------------------------------- |
| Hours vs Needed   | Sleep you got ÷ sleep you needed                       | 55%                                     |
| Sleep Consistency | How similar your bed and wake times are to recent days | 15%                                     |
| Sleep Efficiency  | Time asleep ÷ time in bed                              | 15%                                     |
| Sleep Stress      | Share of the night spent in high stress                | 15%                                     |

WHOOP doesn't publish its weights, so these are a starting point to tune.

**Sleep need**, using WHOOP's published structure:

```
Sleep need = Baseline need + extra for today's Strain + Sleep debt − Naps
```

- Baseline need: learned from your first ~28 days, starting at about 7.5–8 h.
- Strain: harder days add time; scale it with day strain, e.g. 0 below strain 10, rising to about +45 min at 21 (your choice).
- Sleep debt: sleep you needed minus sleep you got on recent nights. NOOP carries 55% of each shortfall forward.
- Naps: subtract nap time.

**Strain (0–21)** — "How hard did I push today?"

- Calculate a training load from time in heart-rate zones (higher zones weighted more), then put it on a log scale: `Strain = 21 × ln(load + 1) ÷ ln(7201)`. Each point gets harder to earn, like WHOOP's.
- Calculate it for the **whole day** and for **each activity**.
- WHOOP's bands: **Light 0–9, Moderate 10–13, High 14–17, All out 18–21.**
- **Strain Target:** a recommended range for today based on Recovery. WHOOP's exact mapping isn't public; a sensible start is green → 14–18, yellow → 10–14, red → 0–10.

**Stress (0–3)** — "How activated is my body right now?"

- Every few minutes, compare current HR and HRV with your 14-day baseline, ignoring periods of movement so exercise isn't counted as stress.
- 0 is calm and 3 is your peak. Choose bands such as low below 1, medium 1–2 and high 2–3.
- **Sleep Stress** = share of sleep spent in your high band. It feeds Sleep Performance.

**Health Monitor status:** for each metric, **green check** = within your normal range, **orange** = slightly outside, **red** = well outside (e.g. beyond 1 and 2 baseline spreads).

**Healthspan (optional, later):** WHOOP Age and Pace of Aging come from nine contributors over about 6 months: sleep duration, sleep consistency, steps, time in zones 1–3, time in zones 4–5, strength activity time, VO₂ max, resting heart rate and lean body mass. WHOOP maps each to published mortality research. A simple version is possible later, but treat it as a rough indicator.

### 9.4 — Storage

```sql
CREATE TABLE daily_summary (
  day TEXT PRIMARY KEY,                 -- 'YYYY-MM-DD' (the day the main sleep ended)
  recovery REAL, recovery_band TEXT,    -- 0–100, 'red'|'yellow'|'green'
  sleep_performance REAL,               -- 0–100
  hours_vs_needed REAL, sleep_consistency REAL, sleep_efficiency REAL, sleep_stress REAL,
  day_strain REAL, strain_target_lo REAL, strain_target_hi REAL,   -- 0–21
  hrv_ms REAL, resting_hr REAL, resp_rate REAL, skin_temp_dev_c REAL, spo2 REAL,
  calories REAL, steps INTEGER, avg_hr REAL,
  zone1_min INTEGER, zone2_min INTEGER, zone3_min INTEGER, zone4_min INTEGER, zone5_min INTEGER,
  confidence TEXT, algo_version INTEGER NOT NULL
);

CREATE TABLE sleeps (
  id TEXT PRIMARY KEY, start_ms INTEGER, end_ms INTEGER, is_nap INTEGER,
  in_bed_min INTEGER, asleep_min INTEGER, latency_min INTEGER, disturbances INTEGER,
  awake_min INTEGER, light_min INTEGER, deep_min INTEGER, rem_min INTEGER,
  need_baseline_min INTEGER, need_strain_min INTEGER, need_debt_min INTEGER, need_naps_min INTEGER,
  resp_rate REAL
);
CREATE TABLE sleep_stages (sleep_id TEXT, start_ms INTEGER, end_ms INTEGER, stage TEXT);

CREATE TABLE activities (
  id TEXT PRIMARY KEY, start_ms INTEGER, end_ms INTEGER, kind TEXT,  -- auto-detected or manual
  strain REAL, avg_hr REAL, max_hr REAL, calories REAL,
  zone1_min INTEGER, zone2_min INTEGER, zone3_min INTEGER, zone4_min INTEGER, zone5_min INTEGER
);

CREATE TABLE stress (ts_ms INTEGER PRIMARY KEY, score REAL);        -- 0–3, every few minutes
CREATE TABLE baselines (day TEXT, metric TEXT, mean REAL, lo REAL, hi REAL, PRIMARY KEY (day, metric));
CREATE TABLE journal (day TEXT, behavior TEXT, answer INTEGER, PRIMARY KEY (day, behavior));  -- 1 yes, 0 no
```

### 9.5 — The screens

Each screen lists **what WHOOP shows**, then **what to build**, with the chart type for each item.

#### Screen 1 — Home

**WHOOP shows:** three dials across the top for **Sleep, Recovery and Strain**, each opening a deep-dive page. Below them: a **Health Monitor** bar (flags any metric outside your range), a **Stress Monitor** graph tile, a **Tonight's Sleep** tile (Sleep Planner), today's **activities** with an **Add Activity** button, key stats, and a **Journal** button.

**Build:**

```
┌──────────────┬───────────────────────────────────────────────────────────┐
│ Home         │  ◀  Thu 20 Nov  ▶                         Synced 07:42 ✓  │
│ Sleep        │                                                           │
│ Recovery     │    ╭───────╮        ╭───────╮        ╭───────╮            │
│ Strain       │    │  88%  │        │  72%  │        │  9.4  │            │
│ Health       │    ╰───────╯        ╰───────╯        ╰───────╯            │
│ Stress       │     SLEEP           RECOVERY          STRAIN              │
│ Trends       │                                      target 14–18         │
│ Journal      │ ┌───────────────────────────────────────────────────────┐ │
│ Assessments  │ │ Health Monitor   5 of 5 metrics in range           ✓  │ │
│              │ └───────────────────────────────────────────────────────┘ │
│ ──────────   │ ┌──────────── Stress ──────────┐ ┌─ Tonight's sleep ────┐ │
│ Strap 84%    │ │ 0–3 line since waking, 1.2 now│ │ Need 8:05 · bed 22:55│ │
│              │ └───────────────────────────────┘ └──────────────────────┘ │
│              │  Activities today        + Add activity                   │
│              │  ▸ Run 07:10–07:52   strain 11.8   avg 148 bpm   512 kcal  │
│              │  HRV 64 ms · RHR 52 · Resp 14.8 · Calories 2,310          │
└──────────────┴───────────────────────────────────────────────────────────┘
```

- **Three dials** (ring charts): Sleep % and Recovery % fill 0–100; Strain fills on a 0–21 scale, with the Strain Target drawn as a band on the ring. Recovery's ring is red, yellow or green.
- **Health Monitor bar:** "All in range" with a check, or the names of metrics out of range with an orange or red marker.
- **Stress tile:** a small 0–3 line chart of today with the current value.
- **Tonight's Sleep tile:** sleep need and recommended bedtime.
- **Activities list:** one row each, with activity strain, time, average HR and calories.
- **Key stats row:** HRV, resting HR, respiratory rate, calories.
- **Date arrows** move through past days.

#### Screen 2 — Sleep

**WHOOP shows:** Sleep Performance and its four contributors (Hours vs Needed, Consistency, Efficiency, Sleep Stress); the four stages (Awake, Light, Deep/SWS, REM); time in bed and asleep; disturbances; restorative sleep; respiratory rate; sleep need and sleep debt; the Sleep Planner; and trends.

**Build:**

1. **Sleep Performance dial** (0–100%), with a **contributor list** below it: four rows, each with its % and a horizontal bar.
2. **Hypnogram:** a step chart of the night (Awake, REM, Light, Deep from top to bottom), with time across the bottom and disturbances marked as ticks.
3. **Stage breakdown:** one stacked horizontal bar plus a table with hours and % per stage. Add a faint typical range for comparison: WHOOP suggests about 50% light, 22% REM, 23% deep and 5% awake.
4. **Restorative sleep:** deep + REM in hours and as a % of time asleep (WHOOP calls 40–50% optimal).
5. **Stats grid:** time in bed, time asleep, efficiency, latency (time to fall asleep), disturbances, respiratory rate, bedtime and wake time.
6. **Sleep need breakdown:** a stacked bar showing Baseline + Strain + Debt − Naps = Need, next to what you got.
7. **Sleep Planner card:** recommended bedtime for tonight at three goals, like WHOOP's: **Peak (100% of need), Perform (85%), Get by (75%)**. Each is your usual wake time minus (need × goal ÷ your typical efficiency).
8. **Consistency chart:** horizontal bars of bed and wake times over the last 14 nights.
9. **Trends link** (Screen 7) for the sleep metrics.

#### Screen 3 — Recovery

**WHOOP shows:** the Recovery % in its colour, and the inputs behind it, chiefly **HRV, resting heart rate and respiratory rate**, each against your own baseline.

**Build:**

1. **Recovery dial** with its band colour and a one-line reading ("Green — your body is primed for strain").
2. **Contributor cards** for HRV, resting HR, respiratory rate and Sleep Performance. Each shows today's value, your baseline, and an arrow that is green when the change is good for recovery and red when it isn't. (Lower is better for resting HR and respiratory rate.)
3. **What moved your score:** a horizontal bar per contributor showing how much it pushed recovery up or down today. WHOOP doesn't show this; it comes almost free from your formula, and it's the most useful thing for learning what affects you.
4. **7-day recovery chart:** daily bars or points coloured red, yellow or green.
5. **Trends link** for Recovery, resting HR, HRV and respiratory rate.

#### Screen 4 — Strain

**WHOOP shows:** **Day Strain** (0–21), the **Strain Target** range based on Recovery, progress toward it in real time, a strain value for **every activity**, activities broken down by **heart-rate zone**, and calories, average HR and steps.

**Build:**

1. **Strain dial** (0–21) with the target range shaded on the ring and today's band label (Light, Moderate, High, All out).
2. **Day HR chart:** 24-hour heart rate line coloured by zone, with activities shaded.
3. **Time in zones:** five horizontal bars (zones 1–5) with minutes for the day.
4. **Activities list:** auto-detected (sustained raised HR) or added manually. Rows show type, time, activity strain, average and max HR, and calories. Let yourself rename the type.
5. **Activity detail:** activity strain, a heart-rate line coloured by zone, zone bars, duration, average and max HR, calories.
6. **Day stats:** calories, average HR, steps (if available), strength activity time (logged manually).
7. **Trends link** for Day Strain, strength activity time, calories, average HR, steps, zones 1–3, zones 4–5 and VO₂ max.

#### Screen 5 — Health Monitor

**WHOOP shows:** live heart rate plus last night's **respiratory rate, SpO2, resting HR, HRV and skin temperature**, each marked as within (green check) or outside (orange or red) your normal range. Each one opens the exact reading, your typical range and possible explanations.

**Build:**

1. **Live heart rate** at the top. This only works while the strap is connected, so show "Live when connected" otherwise.
2. **Five metric rows:** name, last night's value, your normal range (e.g. "Typical 13.9–15.1"), and a status icon.
3. **Metric detail:** a 30-day chart with your normal range as a shaded band and points outside it highlighted, plus a short "what can shift this" note (illness, alcohol, travel, hard training).
4. **Early-warning banner** when several metrics move together (e.g. respiratory rate up, resting HR up, HRV down).

#### Screen 6 — Stress Monitor

**WHOOP shows:** a **0–3** score that updates through the day, a **graph of the day** so you can see when spikes happened, with activities overlaid, plus breathing exercises.

**Build:**

1. **Current stress** value with its band.
2. **Day graph:** a 0–3 line chart from waking until now, with background bands for low, medium and high, and activities and sleep shaded.
3. **Time in each band:** three short bars (low, medium, high) in hours.
4. **Optional:** a guided breathing timer with an animated circle. This is easy in a web UI.

#### Screen 7 — Trends

**WHOOP shows:** weekly, monthly and 6-month trends for these metrics:

- **Sleep:** Sleep Performance, Hours vs Need, Time in Bed, Consistency, Restorative Sleep (%), Restorative Sleep (hours), Efficiency, Sleep Debt
- **Strain:** Day Strain, Strength Activity Time, Calories, Average HR, Steps, Zones 1–3, Zones 4–5, VO₂ Max
- **Recovery:** Recovery, Resting HR, HRV, Respiratory Rate

**Build:**

1. A metric picker grouped Sleep / Strain / Recovery with exactly that list (plus Stress, if you like).
2. A range switch: **1W / 1M / 6M**, plus 1Y because your data is all local anyway.
3. **Chart:** daily bars for 1W and 1M; a line of weekly averages for 6M and 1Y. Recovery bars are coloured by band. Show your baseline band where one exists.
4. A summary line: average for the period, change from the previous period, best and worst day.

#### Screen 8 — Journal and Insights

**WHOOP shows:** a daily journal of yes/no behaviours (WHOOP offers 300+), and **Behavior Insights** showing how each behaviour relates to your Recovery. A behaviour unlocks once it has at least 5 "yes" and 5 "no" answers within 90 days.

**Build:**

1. **Daily check-in:** a short list of yes/no questions you pick from a library (alcohol, caffeine after 2 pm, late meal, screens in bed, stretching, sauna, magnesium, travel, illness…). Ask about yesterday, since it affects today's recovery.
2. **Insights:** for each behaviour with ≥5 yes and ≥5 no in the last 90 days, the difference in average next-day recovery (yes vs no). Show it as a horizontal bar chart centred on zero: green bars helped, red bars hurt. Label each with "+6%" or "−9%" and the number of days.
3. Add the same for Sleep Performance and HRV if you want more than WHOOP shows.
4. With few data points, show "Not enough data yet (3 yes / 7 no)" rather than a misleading number.

#### Screen 9 — Performance Assessments

**WHOOP shows:** weekly and monthly reports. The monthly one includes sleep analysis, strain and time in zones by activity, **training and response** (strain vs recovery over 6 months, alongside cardiovascular fitness), and how journal behaviours affected recovery. The annual one uses **month-by-month heat maps** of Strain, Recovery and Sleep.

**Build:**

1. **Weekly:** average recovery, strain and sleep; a **scatter plot of day strain vs next-day recovery**; sleep consistency (bed/wake time ranges); total hours slept vs needed.
2. **Monthly:** the weekly content plus strain and zone time by activity type, the top journal impacts, and a 6-month **strain vs recovery time series** with resting HR or VO₂ max underneath.
3. **Yearly:** calendar heat maps (one square per day) for Recovery, Strain and Sleep.
4. Create them automatically each Monday and on the 1st of the month, with a notification, and keep past reports in a list.

#### Screen 10 — Healthspan (optional, last)

**WHOOP shows:** **WHOOP Age** (physiological age vs actual age) and **Pace of Aging** (how fast that's changing, updated weekly), driven by nine contributors.

**Build (approximate):** a big "age" number next to your real age; a pace gauge; and nine contributor rows (sleep duration, sleep consistency, steps, zones 1–3, zones 4–5, strength time, VO₂ max, resting HR, lean body mass), each with your 6-month average, a target, and whether it adds or removes years. Only build this once everything else works, and treat its output as a motivator, not a measurement.

### 9.6 — Building the components

- **Dials:** SVG rings drawn with `stroke-dasharray`. Recovery is coloured by band; Sleep uses one fixed colour; Strain's ring runs 0–21 with the target band drawn as a lighter arc.
- **Charts:** uPlot for long heart-rate and stress lines; ECharts or Recharts for the hypnogram (step line), stacked stage bar, zone bars, scatter plots and calendar heat maps (ECharts has a built-in calendar heat map).
- **One Rust command per screen** (`get_home(day)`, `get_sleep(day)`, `get_trend(metric, range)`…) returning only what that screen draws.
- **Glass:** glass on the window and sidebar; slightly solid cards behind dials and charts so they stay readable. Check light and dark mode.
- **Empty states:** "Calibrating — 2 more nights", "Strap was off 02:10–04:30", never a fake zero.
- **Live updates:** after each automatic sync, recalculate affected days and send an event so open screens refresh.

### 9.7 — Check the numbers

- Write unit tests for every score with hand-worked examples.
- If you have a WHOOP data export, import it. Chart your Recovery, Sleep Performance and Strain against WHOOP's for the same days. Aim for the same shape and direction; large gaps usually mean a bug in sleep detection or baselines.
- These are estimates from a wrist sensor, not medical measurements.

**Done when:** every morning, Home shows your three dials, and each screen shows the same information WHOOP's equivalent does.

---

## Phase 10 — Build it as a real app

1. `npm run tauri build` produces a `.app` and a `.dmg` in `src-tauri/target/release/bundle/`.
2. Without an Apple Developer account, macOS will warn the first time you open it. Right-click the app → Open, or allow it in System Settings → Privacy & Security. You only do this once per version.
3. Move it to `/Applications` so login-start and Bluetooth permission stick to the installed copy.

**Done when:** the installed app in `/Applications` syncs on its own.

---

## Phase 11 (optional) — Backups and extras

- **Backups:** the database is one file. Time Machine already covers it. You can also add a menu item that copies it to iCloud Drive or another folder.
- **Sync server:** if you later want the data on another computer, add a small server and upload only new `raw_frames` rows, using each row's unique ID to avoid duplicates.
- **Phone app:** if you later want live readings away from your desk, reuse `whoop-core` in a Tauri iOS app. The Bluetooth part on iPhone is best written in Swift (see the earlier iPhone plan).

---

## Risks and how to handle them

| Risk                                       | What happens                                                   | What to do                                                                       |
| ------------------------------------------ | -------------------------------------------------------------- | -------------------------------------------------------------------------------- |
| More than ~14 days without a sync          | The oldest data is overwritten on the strap                    | Overdue notifications (Phase 7); sync before holidays                            |
| Official WHOOP app still syncing           | It may take history before your Mac does, and pairing can fail | Use one or the other; keep the strap away from the official app while pairing    |
| Strap firmware update changes the protocol | Decoding fails or gives wrong values                           | Raw packets are stored, so fix the decoder and rebuild; watch NOOP's changelog   |
| macOS update changes the private glass API | The glass effect breaks or looks odd                           | Turn it off with `enabled: false`; the app works without it                      |
| Mac asleep or away from you                | No sync that day                                               | Fine: the strap holds two weeks, and the next sync catches up                    |
| Your health data on disk                   | Private data in a plain file                                   | FileVault encryption on the Mac covers it; don't put the file in a shared folder |

---

## Milestone checklist

- [ ] Read NOOP's protocol docs and identified my strap (4.0 or 5.0)
- [ ] Tauri app runs on my Mac
- [ ] Liquid Glass window working
- [ ] Raw bytes from the strap appear in the app
- [ ] Live heart rate decoded and shown
- [ ] Readings saved and still there after relaunching
- [ ] A full day of history downloads from the strap
- [ ] Menu bar app syncs automatically, starts at login, warns when overdue
- [ ] Dashboard shows today, last night and trends
- [ ] Raw measurements: HRV, resting HR, respiratory rate, sleep stages, zones, live stress
- [ ] Personal baselines working ("Calibrating" for the first nights)
- [ ] Recovery (0–100%), Sleep Performance (0–100%), Strain (0–21) and Stress (0–3) calculated
- [ ] Home screen with the three dials, Health Monitor bar, Stress tile, Tonight's Sleep and activities
- [ ] Sleep, Recovery and Strain deep-dive screens
- [ ] Health Monitor and Stress Monitor screens
- [ ] Trends (1W / 1M / 6M) for every WHOOP trend metric
- [ ] Journal with Behavior Insights
- [ ] Weekly and monthly Performance Assessments
- [ ] (Optional) Healthspan
- [ ] Installed in /Applications and running on its own

---

## Sources

- NOOP build guide: https://github.com/ryanbr/noop/blob/main/docs/BUILD.md
- NOOP repository (protocol docs in `docs/`, license in `LICENSE`): https://github.com/ryanbr/noop
- tauri-plugin-liquid-glass: https://github.com/hkandala/tauri-plugin-liquid-glass (docs: https://docs.rs/crate/tauri-plugin-liquid-glass/latest)
- btleplug (Rust Bluetooth LE): https://github.com/deviceplug/btleplug
- tauri-plugin-blec: https://github.com/MnlPhlp/tauri-plugin-blec
- Tauri 2 prerequisites: https://v2.tauri.app/start/prerequisites/
- NOOP's score methods (recovery, strain, sleep, baselines): https://github.com/ryanbr/noop/blob/main/docs/ANALYTICS.md

**How the WHOOP app presents each screen (used for Phase 9):**

- Home screen redesign (Sleep, Recovery, Strain dials): https://www.whoop.com/us/en/thelocker/the-all-new-whoop-home-screen/
- Earlier Overview screen (Health and Stress Monitor tiles, Add Activity): https://www.whoop.com/thelocker/your-key-whoop-metrics-all-in-one-place
- Recovery inputs, Strain and Stress scales: https://www.whoop.com/de/en/the-data/
- Sleep Performance contributors and stages: https://www.whoop.com/the-locker/how-well-whoop-measures-sleep/
- Sleep need and Sleep Planner: https://www.whoop.com/us/en/thelocker/how-much-sleep-do-i-need
- Sleep stage percentages: https://www.whoop.com/the-locker/what-is-deep-sleep/
- Restorative sleep: https://www.whoop.com/thelocker/understand-everything-that-makes-a-great-nights-sleep-with-new-sleep-metrics
- Strain bands (0–21): https://www.whoop.com/au/en/thelocker/how-does-whoop-strain-work-101
- Strain Target: https://www.whoop.com/kr/en/thelocker/strain-coach
- Health Monitor: https://www.whoop.com/ae/en/thelocker/health-monitor-feature/
- Stress Monitor: https://www.whoop.com/ca/en/thelocker/introducing-stress-monitor-a-new-way-to-monitor-manage-stress/
- Trend views: https://www.whoop.com/gg/en/thelocker/track-progress-with-new-trend-views/
- Journal and Behavior Insights: https://www.whoop.com/us/fr/thelocker/the-whoop-journal/
- Monthly Performance Assessment: https://www.whoop.com/thelocker/monthly-performance-assessment
- Healthspan: https://www.whoop.com/ca/en/thelocker/Healthspan-Data-Meets-Longevity/
- WHOOP storing up to 14 days of unsynced data: https://community.whoop.com/t/how-often-does-whoop-wearable-makes-connection-to-the-phone-to-synch-data/1987
