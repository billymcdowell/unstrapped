/**
 * Response types for every Tauri command and event. This file is the contract
 * between the React front end and the Rust backend; see docs/API_CONTRACT.md.
 *
 * Conventions
 * - Rust structs use `#[serde(rename_all = "camelCase")]`, so field names here are exact JSON keys.
 * - Days are local-date strings "YYYY-MM-DD". Instants are Unix milliseconds (number).
 * - Time series are columnar: `{ ts: number[]; v: (number|null)[] }` (null = no reading / gap).
 * - Anything that can be unavailable is wrapped in `Result<T>`; never send fake zeros.
 */

export type Day = string; // "YYYY-MM-DD"
export type Ms = number; // Unix ms

// ---------------------------------------------------------------- shared states

/** A period without usable data, e.g. "Strap was off 02:10-04:30". */
export interface Gap {
  startMs: Ms;
  endMs: Ms;
  /** Why, e.g. "Strap was off" / "Strap out of range". */
  reason: string;
}

export interface Calibrating {
  state: "calibrating";
  nightsRemaining: number;
  message?: string;
}

export interface NoData {
  state: "noData";
  reason: string;
  gaps: Gap[];
}

export interface Ok<T> {
  state: "ok";
  data: T;
}

/**
 * Rust: `#[serde(tag = "state", rename_all = "camelCase")] enum Result<T> {
 *   Ok { data: T }, Calibrating { nights_remaining: u32, message: Option<String> },
 *   NoData { reason: String, gaps: Vec<Gap> } }`
 */
export type Result<T> = Ok<T> | Calibrating | NoData;

export interface TimeSeries {
  ts: Ms[];
  v: (number | null)[];
}

export type Band = "red" | "yellow" | "green";
export type StrainBand = "light" | "moderate" | "high" | "allOut";
export type StressBand = "low" | "medium" | "high";
export type StatusLevel = "ok" | "warn" | "alert" | "unavailable";
export type SleepStage = "awake" | "light" | "deep" | "rem";

export interface Span {
  startMs: Ms;
  endMs: Ms;
  kind: "sleep" | "nap" | "activity" | "gap";
  label?: string;
}

// ---------------------------------------------------------------- activities

export interface ActivitySummary {
  id: string;
  /** User-editable label, e.g. "Run". */
  kind: string;
  startMs: Ms;
  endMs: Ms;
  strain: number; // 0-21
  avgHr: number;
  maxHr: number;
  calories: number;
  /** Minutes in zones 1..5. */
  zoneMin: [number, number, number, number, number];
  /** true = added by the user, false = auto-detected. */
  manual: boolean;
}

export interface ActivityDetail {
  summary: ActivitySummary;
  hr: TimeSeries;
  /** Lower bpm bound of zones 2..5 (4 numbers); zone 1 starts at the first one minus its width. */
  zoneBounds: [number, number, number, number];
  maxHr: number;
}

// ---------------------------------------------------------------- home

export interface HealthMonitorSummary {
  inRange: number;
  total: number;
  flagged: { metric: HealthMetricKey; label: string; status: StatusLevel }[];
}

export interface HomeData {
  day: Day;
  sleep: Result<{ performance: number }>;
  recovery: Result<{ score: number; band: Band }>;
  strain: Result<{ value: number; band: StrainBand; targetLo: number | null; targetHi: number | null }>;
  healthMonitor: Result<HealthMonitorSummary>;
  stress: Result<{ current: number | null; band: StressBand | null; series: TimeSeries }>;
  tonightsSleep: Result<{ needMin: number; recommendedBedtimeMs: Ms; wakeTimeMs: Ms }>;
  activities: ActivitySummary[];
  keyStats: Result<{ hrvMs: number | null; restingHr: number | null; respRate: number | null; calories: number | null }>;
  /** Gaps in today's data that explain missing numbers. */
  gaps: Gap[];
}

// ---------------------------------------------------------------- sleep

export interface SleepContributor {
  key: "hoursVsNeeded" | "consistency" | "efficiency" | "sleepStress";
  label: string;
  pct: number | null; // 0-100; null = not available (e.g. consistency needs several nights)
}

export interface SleepStageSegment {
  startMs: Ms;
  endMs: Ms;
  stage: SleepStage;
}

export interface SleepNeed {
  baselineMin: number;
  strainMin: number;
  debtMin: number;
  /** Positive number; it is subtracted. */
  napsMin: number;
  totalMin: number;
  actualMin: number;
}

export interface SleepPlannerOption {
  goal: "peak" | "perform" | "getBy";
  /** 100 / 85 / 75 */
  goalPct: number;
  bedtimeMs: Ms;
  sleepMin: number;
}

export interface SleepData {
  day: Day;
  sleepId: string;
  performance: number; // 0-100
  contributors: SleepContributor[]; // always the four WHOOP-style contributors
  segments: SleepStageSegment[]; // hypnogram, 30 s granularity merged into runs
  disturbanceTimesMs: Ms[];
  stageMin: { awake: number; light: number; deep: number; rem: number };
  /** Typical share of the night, percent (50/22/23/5 by default). */
  typicalPct: { awake: number; light: number; deep: number; rem: number };
  restorative: { min: number; pctOfAsleep: number };
  stats: {
    timeInBedMin: number;
    asleepMin: number;
    efficiencyPct: number;
    latencyMin: number;
    disturbances: number;
    respRate: number | null;
    bedtimeMs: Ms;
    wakeMs: Ms;
  };
  need: SleepNeed;
  planner: Result<{ wakeTimeMs: Ms; typicalEfficiencyPct: number; options: SleepPlannerOption[] }>;
  /** Last 14 nights. */
  consistency: { day: Day; bedMs: Ms; wakeMs: Ms }[];
  naps: { startMs: Ms; endMs: Ms }[];
  gaps: Gap[];
}

// ---------------------------------------------------------------- recovery

export interface RecoveryContributor {
  key: "hrv" | "restingHr" | "respRate" | "sleepPerformance";
  label: string;
  value: number;
  unit: string; // "ms" | "bpm" | "rpm" | "%"
  baseline: number;
  /** Change vs baseline in percent (positive = higher than baseline). */
  deltaPct: number;
  /** Whether the change is good for recovery (lower is better for restingHr/respRate). */
  direction: "good" | "bad" | "neutral";
  /** How many recovery points this input added (+) or removed (-) today. */
  impactPts: number;
}

export interface RecoveryData {
  day: Day;
  score: number; // 0-100
  band: Band;
  reading: string; // "Green: your body is primed for strain."
  contributors: RecoveryContributor[];
  last7: { day: Day; score: number | null; band: Band | null }[];
}

// ---------------------------------------------------------------- strain

export interface StrainData {
  day: Day;
  value: number; // 0-21
  band: StrainBand;
  targetLo: number | null;
  targetHi: number | null;
  /** Progress toward the target as percent of targetLo, null if no target. */
  hr: TimeSeries; // per-minute bpm for the day
  zoneBounds: [number, number, number, number];
  maxHr: number;
  zoneMin: [number, number, number, number, number];
  spans: Span[]; // sleeps, naps, activities, gaps
  activities: ActivitySummary[];
  stats: { calories: number | null; avgHr: number | null; steps: number | null; strengthMin: number };
  gaps: Gap[];
}

// ---------------------------------------------------------------- health monitor

export type HealthMetricKey = "respRate" | "spo2" | "restingHr" | "hrv" | "skinTemp";

export interface HealthMetric {
  key: HealthMetricKey;
  label: string;
  value: number | null;
  unit: string;
  rangeLo: number | null;
  rangeHi: number | null;
  status: StatusLevel;
}

export interface HealthData {
  day: Day;
  /** null when the strap has no live reading. */
  liveHr: number | null;
  metrics: Result<HealthMetric[]>;
  earlyWarning: { message: string; metrics: HealthMetricKey[] } | null;
}

export interface MetricDetail {
  metric: HealthMetricKey;
  label: string;
  unit: string;
  current: number | null;
  rangeLo: number | null;
  rangeHi: number | null;
  status: StatusLevel;
  /** Last 30 days. */
  points: { day: Day; value: number | null; lo: number | null; hi: number | null }[];
  note: string; // what can shift this
}

// ---------------------------------------------------------------- stress

export interface StressData {
  day: Day;
  current: number | null; // 0-3
  band: StressBand | null;
  /** 0-3, every few minutes since waking. */
  series: TimeSeries;
  /** Band edges: low < lowMax <= medium < mediumMax <= high (default 1 and 2). */
  thresholds: { lowMax: number; mediumMax: number };
  hoursInBand: { low: number; medium: number; high: number };
  spans: Span[];
  gaps: Gap[];
}

// ---------------------------------------------------------------- trends

export type TrendRange = "1W" | "1M" | "6M" | "1Y";

export type TrendMetric =
  // Sleep
  | "sleepPerformance" | "hoursVsNeed" | "timeInBed" | "consistency" | "restorativePct"
  | "restorativeHours" | "efficiency" | "sleepDebt"
  // Strain
  | "dayStrain" | "strengthTime" | "calories" | "avgHr" | "steps" | "zones123" | "zones45" | "vo2max"
  // Recovery
  | "recovery" | "restingHr" | "hrv" | "respRate"
  // Extra
  | "stress";

export interface TrendData {
  metric: TrendMetric;
  label: string;
  unit: string;
  range: TrendRange;
  /** Daily for 1W/1M, weekly averages for 6M/1Y. `day` is the first day of the bucket. */
  granularity: "daily" | "weekly";
  points: { day: Day; value: number | null; band: Band | null }[];
  baseline: { lo: number; hi: number } | null;
  summary: {
    avg: number;
    changePct: number | null;
    best: { day: Day; value: number };
    worst: { day: Day; value: number };
  } | null;
}

// ---------------------------------------------------------------- journal

export interface JournalQuestion {
  id: string;
  label: string;
  category: string;
  /** null = unanswered. */
  answer: boolean | null;
}

export interface JournalDay {
  day: Day;
  questions: JournalQuestion[];
}

export interface JournalLibraryItem {
  id: string;
  label: string;
  category: string;
  selected: boolean;
}

export interface Insight {
  behavior: string;
  label: string;
  yesDays: number;
  noDays: number;
  /** "ok" when >= 5 yes and >= 5 no in 90 days. */
  state: "ok" | "notEnough";
  /** Difference in average next-day recovery (yes minus no), percentage points. */
  recoveryDelta: number | null;
  sleepDelta: number | null;
  hrvDelta: number | null;
}

// ---------------------------------------------------------------- assessments

export type AssessmentKind = "weekly" | "monthly" | "yearly";

export interface AssessmentListItem {
  id: string;
  kind: AssessmentKind;
  title: string;
  periodStart: Day;
  periodEnd: Day;
  createdMs: Ms;
}

export interface Assessment {
  id: string;
  kind: AssessmentKind;
  title: string;
  periodStart: Day;
  periodEnd: Day;
  avgRecovery: number | null;
  avgStrain: number | null;
  avgSleepPerformance: number | null;
  hoursSlept: number;
  hoursNeeded: number;
  /** Day strain vs next-day recovery. */
  scatter: { day: Day; strain: number; nextRecovery: number }[];
  consistency: { day: Day; bedMs: Ms; wakeMs: Ms }[];
  /** Monthly and yearly only. */
  zonesByActivity?: { kind: string; zoneMin: [number, number, number, number, number]; /** Added: mean activity strain, optional. */ avgStrain?: number | null; /** Added: number of activities, optional. */ sessions?: number }[];
  topJournalImpacts?: Insight[];
  trainingResponse?: { months: string[]; strain: (number | null)[]; recovery: (number | null)[]; restingHr: (number | null)[] };
  /** Yearly only: one value per day. */
  heat?: { recovery: { day: Day; value: number }[]; strain: { day: Day; value: number }[]; sleep: { day: Day; value: number }[] };
}

// ---------------------------------------------------------------- healthspan

export interface HealthspanContributor {
  key: string;
  label: string;
  /** 6-month average, display string with unit. */
  average: string;
  target: string;
  /** Years added (+) or removed (-). */
  yearsImpact: number;
}

export interface Healthspan {
  strapAge: number;
  chronologicalAge: number;
  /** Years of biological ageing per calendar year; 1.0 = normal. */
  paceOfAging: number;
  contributors: HealthspanContributor[];
}

// ---------------------------------------------------------------- raw dashboard (Phase 8)

export interface RawDay {
  day: Day;
  /** Per-minute heart rate across the 24 h of `day`. */
  hr: TimeSeries;
  spans: Span[];
}

export interface RawNight {
  day: Day;
  hr: TimeSeries;
  /** R-R intervals: `ts` of each beat, `v` interval in ms. */
  rr: TimeSeries;
}

export interface RawTrends {
  range: TrendRange;
  days: { day: Day; restingHr: number | null; hrv: number | null }[];
}

export type ExportKind = "heart_rate" | "rr_intervals" | "daily_summary" | "sleeps" | "activities" | "stress" | "raw_frames";

// ---------------------------------------------------------------- strap

export type Transport = "ble" | "emulator";

export type StrapState = "disconnected" | "scanning" | "connecting" | "connected" | "syncing" | "error";

export interface StrapStatus {
  transport: Transport;
  state: StrapState;
  deviceName: string | null;
  batteryPct: number | null;
  charging: boolean;
  /** null = unknown. */
  onWrist: boolean | null;
  lastSyncMs: Ms | null;
  lastHr: number | null;
  error: string | null;
}

export interface SyncProgress {
  phase: "idle" | "scanning" | "connecting" | "requesting" | "downloading" | "processing" | "done" | "error";
  /** 0-1 */
  fraction: number;
  /** "Downloading 23 h of data… 64%" */
  label: string;
}

export interface RawFrameEvent {
  receivedAt: Ms;
  characteristic: string;
  hex: string;
}

export interface HrEvent {
  ts: Ms;
  bpm: number;
}

export interface DataUpdatedEvent {
  days: Day[];
}

export type Sex = "male" | "female" | "other";

export interface Settings {
  profile: { age: number; sex: Sex; weightKg: number; heightCm: number };
  transport: Transport;
  /** "host:port", default "127.0.0.1:47800". */
  emulatorAddress: string;
  autoSync: boolean;
  launchAtLogin: boolean;
}

export type EmulatorAction = "out_of_range" | "in_range" | "take_off" | "put_on" | "advance_minutes";
