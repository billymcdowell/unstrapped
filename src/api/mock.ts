/**
 * Deterministic mock backend used automatically outside Tauri (plain `npm run dev`).
 * Same command names / arg keys / return shapes as the real Rust commands.
 * Data is seeded from the day string, so a given day always looks the same.
 */
import { addDays, dayStartMs, daysBetween, today, toDay, parseDay } from "../lib/date";
import type {
  ActivityDetail, ActivitySummary, Assessment, Band, Day, Gap, HealthMetric, HealthMetricKey, Insight, JournalLibraryItem,
  Ms, Result, SleepStage, SleepStageSegment, Span, StatusLevel, StrapStatus, Settings, SyncProgress, TimeSeries,
  TrendMetric, TrendRange, StressBand, StrainBand,
} from "./types";

// ------------------------------------------------------------------ rng

function hash(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
  return h >>> 0;
}
function rng(seed: number) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
const round = (n: number, d = 0) => { const f = 10 ** d; return Math.round(n * f) / f; };
const clamp = (n: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, n));
const MIN = 60_000;

const FIRST_DATA_DAYS = 45; // history available
const MAX_HR = 190;
const profile = { age: 34 };

// ------------------------------------------------------------------ event bus

const listeners = new Map<string, Set<(p: unknown) => void>>();
export function subscribe(event: string, cb: (p: unknown) => void): () => void {
  if (!listeners.has(event)) listeners.set(event, new Set());
  const wrapped = (p: unknown) => cb(p); // unique identity so React StrictMode double-subscribes don't collide
  listeners.get(event)!.add(wrapped);
  return () => { listeners.get(event)?.delete(wrapped); };
}
function emit(event: string, payload: unknown) {
  listeners.get(event)?.forEach((cb) => cb(payload));
}

// ------------------------------------------------------------------ per-day model

interface DayCore {
  day: Day;
  hasData: boolean;
  recovery: number;
  hrv: number;
  rhr: number;
  resp: number;
  skinTemp: number;
  spo2: number;
  bedMs: Ms;
  wakeMs: Ms;
  sleepPerf: number;
  efficiency: number;
  consistency: number;
  sleepStress: number;
  hoursVsNeeded: number;
  needMin: number;
  asleepMin: number;
  strain: number;
  steps: number;
  calories: number;
  avgHr: number;
  zoneMin: [number, number, number, number, number];
  activities: ActivitySummary[];
  strapOff: Gap | null;
}

const coreCache = new Map<Day, DayCore>();
const recBand = (s: number): Band => (s >= 67 ? "green" : s >= 34 ? "yellow" : "red");
const strainBandOf = (s: number): StrainBand => (s >= 18 ? "allOut" : s >= 14 ? "high" : s >= 10 ? "moderate" : "light");
const stressBandOf = (s: number): StressBand => (s < 1 ? "low" : s < 2 ? "medium" : "high");

function zoneBounds(rhr: number): [number, number, number, number] {
  const hrr = MAX_HR - rhr;
  return [0.6, 0.7, 0.8, 0.9].map((p) => Math.round(rhr + p * hrr)) as [number, number, number, number];
}
function strainFromZones(z: number[]): number {
  const load = z.reduce((s, m, i) => s + m * (i + 1) * 0.6, 0);
  return clamp((21 * Math.log(load + 1)) / Math.log(7201), 0, 21);
}

function core(day: Day): DayCore {
  const hit = coreCache.get(day);
  if (hit) return hit;
  const r = rng(hash(day));
  const age = daysBetween(day, today());
  const hasData = age >= 0 && age <= FIRST_DATA_DAYS;
  // slow weekly wave + noise
  const wave = Math.sin(hash(day) % 7) * 0.4 + (r() - 0.5);
  const recovery = Math.round(clamp(62 + wave * 28, 4, 99));
  const hrv = round(clamp(62 + (recovery - 60) * 0.5 + (r() - 0.5) * 8, 28, 110), 0);
  const rhr = round(clamp(53 - (recovery - 60) * 0.06 + (r() - 0.5) * 3, 44, 70), 0);
  const resp = round(14.8 + (r() - 0.5) * 1.2 - (recovery - 60) * 0.005, 1);
  const bedClock = 22 * 60 + 40 + Math.round((r() - 0.5) * 100); // minutes after previous midnight
  const bedMs = dayStartMs(addDays(day, -1)) + bedClock * MIN;
  const inBed = Math.round(7.4 * 60 + (r() - 0.45) * 100);
  const wakeMs = bedMs + inBed * MIN;
  const efficiency = round(clamp(90 + (r() - 0.5) * 10, 78, 98), 0);
  const asleepMin = Math.round((inBed * efficiency) / 100);
  const needMin = Math.round(8 * 60 + (r() - 0.5) * 40);
  const hoursVsNeeded = clamp(Math.round((asleepMin / needMin) * 100), 30, 100);
  const consistency = Math.round(clamp(82 + (r() - 0.5) * 30, 50, 100));
  const sleepStress = Math.round(clamp(88 + (r() - 0.5) * 22, 55, 100));
  const sleepPerf = Math.round(hoursVsNeeded * 0.55 + consistency * 0.15 + efficiency * 0.15 + sleepStress * 0.15);

  const activities: ActivitySummary[] = [];
  const nAct = r() < 0.35 ? 0 : r() < 0.85 ? 1 : 2;
  const rhrNow = rhr;
  const zb = zoneBounds(rhrNow);
  for (let i = 0; i < nAct; i++) {
    const kinds = ["Run", "Strength", "Cycling", "Walk", "Yoga"];
    const kind = kinds[Math.floor(r() * kinds.length)];
    const start = dayStartMs(day) + (i === 0 ? 7 * 60 + 10 + Math.round(r() * 60) : 17 * 60 + 30 + Math.round(r() * 60)) * MIN;
    const dur = Math.round(25 + r() * 50);
    const intensity = kind === "Yoga" || kind === "Walk" ? 0.55 : 0.8;
    const zoneMin: [number, number, number, number, number] = [0, 0, 0, 0, 0];
    let sum = 0;
    for (let m = 0; m < dur; m++) {
      const bpm = rhrNow + (MAX_HR - rhrNow) * clamp(intensity * (0.7 + 0.5 * Math.sin(m / 6) * 0.3 + r() * 0.25), 0.2, 0.98);
      sum += bpm;
      const z = Math.min(4, Math.max(0, bpm >= zb[3] ? 4 : bpm >= zb[2] ? 3 : bpm >= zb[1] ? 2 : bpm >= zb[0] ? 1 : bpm >= rhrNow + 0.5 * (MAX_HR - rhrNow) ? 0 : -1));
      if (z >= 0) zoneMin[z]++;
    }
    const avg = Math.round(sum / dur);
    activities.push({
      id: `act-${day}-${i}`, kind, startMs: start, endMs: start + dur * MIN,
      strain: round(strainFromZones(zoneMin), 1), avgHr: avg, maxHr: Math.round(avg + 18 + r() * 12),
      calories: Math.round(dur * (7 + intensity * 7)), zoneMin, manual: i === 1 && r() < 0.3,
    });
  }
  const zoneMin: [number, number, number, number, number] = [0, 0, 0, 0, 0];
  const base = [55 + r() * 40, 30 + r() * 25, 10 + r() * 15, 3 + r() * 8, r() * 4];
  base.forEach((v, i) => (zoneMin[i] = Math.round(v) + activities.reduce((s, a) => s + a.zoneMin[i], 0)));
  const strain = round(strainFromZones(zoneMin), 1);
  const strapOff: Gap | null = hash(day) % 4 === 0
    ? { startMs: dayStartMs(day) + (14 * 60 + 5) * MIN, endMs: dayStartMs(day) + (14 * 60 + 50) * MIN, reason: "Strap was off (charging)" }
    : null;

  const c: DayCore = {
    day, hasData, recovery, hrv, rhr, resp,
    skinTemp: round((r() - 0.5) * 0.8, 1), spo2: round(96.5 + r() * 1.8, 1),
    bedMs, wakeMs, sleepPerf, efficiency, consistency, sleepStress, hoursVsNeeded, needMin, asleepMin,
    strain, steps: Math.round(5200 + r() * 7000 + activities.length * 2500),
    calories: Math.round(2050 + strain * 52 + r() * 120), avgHr: Math.round(rhr + 14 + strain),
    zoneMin, activities, strapOff,
  };
  coreCache.set(day, c);
  return c;
}

function noData(day: Day): Result<never> {
  const age = daysBetween(day, today());
  if (age < 0) return { state: "noData", reason: "This day hasn't happened yet.", gaps: [] };
  const s = dayStartMs(day);
  return {
    state: "noData",
    reason: "No data for this day. Sync your strap to fill it in.",
    gaps: [{ startMs: s, endMs: s + 86_400_000, reason: "No strap data" }],
  };
}
const ok = <T,>(data: T): Result<T> => ({ state: "ok", data });

function dayGaps(c: DayCore): Gap[] {
  return c.strapOff ? [c.strapOff] : [];
}

// ------------------------------------------------------------------ series

function hrSeries(c: DayCore, withNulls = true): TimeSeries {
  const r = rng(hash(c.day + "hr"));
  const start = dayStartMs(c.day);
  const isToday = c.day === today();
  const endMin = isToday ? Math.max(60, Math.floor((Date.now() - start) / MIN)) : 1440;
  const ts: Ms[] = [];
  const v: (number | null)[] = [];
  for (let m = 0; m < endMin; m++) {
    const t = start + m * MIN;
    let bpm: number;
    if (t < c.wakeMs) {
      const phase = (t - c.bedMs) / (c.wakeMs - c.bedMs);
      bpm = c.rhr + 4 + 5 * Math.sin(phase * Math.PI * 5) ** 2 + r() * 3 - (phase > 0.1 && phase < 0.5 ? 2 : 0);
    } else {
      const hod = m / 60;
      bpm = c.rhr + 16 + 8 * Math.sin(hod / 2.2) + r() * 9;
      const act = c.activities.find((a) => t >= a.startMs && t < a.endMs);
      if (act) {
        const p = (t - act.startMs) / (act.endMs - act.startMs);
        bpm = act.avgHr + 14 * Math.sin(p * 9) + (r() - 0.5) * 8;
      }
    }
    ts.push(t);
    v.push(withNulls && c.strapOff && t >= c.strapOff.startMs && t < c.strapOff.endMs ? null : Math.round(bpm));
  }
  return { ts, v };
}

function stressSeries(c: DayCore): TimeSeries {
  const r = rng(hash(c.day + "st"));
  const ts: Ms[] = [];
  const v: (number | null)[] = [];
  const end = c.day === today() ? Date.now() : dayStartMs(c.day) + 86_400_000;
  for (let t = c.wakeMs; t < end; t += 5 * MIN) {
    const hod = (t - dayStartMs(c.day)) / 3_600_000;
    let s = 0.9 + 0.7 * Math.sin(hod / 2.1) + (r() - 0.5) * 0.7;
    if (c.activities.some((a) => t >= a.startMs && t < a.endMs)) s = NaN;
    ts.push(t);
    v.push(Number.isNaN(s) || (c.strapOff && t >= c.strapOff.startMs && t < c.strapOff.endMs) ? null : round(clamp(s, 0.1, 2.9), 2));
  }
  return { ts, v };
}

function stageSegments(c: DayCore): SleepStageSegment[] {
  const r = rng(hash(c.day + "stg"));
  const segs: SleepStageSegment[] = [];
  let t = c.bedMs;
  const push = (stage: SleepStage, min: number) => {
    const end = Math.min(c.wakeMs, t + min * MIN);
    if (end > t) segs.push({ startMs: t, endMs: end, stage });
    t = end;
  };
  push("awake", 8 + r() * 10);
  let cycle = 0;
  while (t < c.wakeMs) {
    const early = cycle < 2;
    push("light", 10 + r() * 15);
    push("deep", early ? 22 + r() * 25 : 4 + r() * 10);
    push("light", 8 + r() * 14);
    push("rem", early ? 8 + r() * 10 : 18 + r() * 22);
    if (r() < 0.45) push("awake", 2 + r() * 5);
    cycle++;
  }
  return segs;
}

function minutesBy(segs: SleepStageSegment[]) {
  const m = { awake: 0, light: 0, deep: 0, rem: 0 };
  segs.forEach((s) => (m[s.stage] += Math.round((s.endMs - s.startMs) / MIN)));
  return m;
}

// ------------------------------------------------------------------ screens

function activitiesFor(day: Day) {
  const extra = manualActivities.filter((a) => toDay(new Date(a.startMs)) === day);
  return [...core(day).activities, ...extra].map((a) => renamed.get(a.id) ?? a).sort((a, b) => a.startMs - b.startMs);
}
const manualActivities: ActivitySummary[] = [];
const renamed = new Map<string, ActivitySummary>();

function home(day: Day) {
  const c = core(day);
  if (!c.hasData) {
    const n = noData(day);
    return { day, sleep: n, recovery: n, strain: n, healthMonitor: n, stress: n, tonightsSleep: n, activities: [], keyStats: n, gaps: [] };
  }
  const strainTarget = c.recovery >= 67 ? [14, 18] : c.recovery >= 34 ? [10, 14] : [0, 10];
  const st = stressSeries(c);
  const cur = [...st.v].reverse().find((x) => x !== null) ?? null;
  const hm = healthMetrics(c);
  const flagged = hm.filter((m) => m.status === "warn" || m.status === "alert");
  const wake = c.wakeMs;
  return {
    day,
    sleep: ok({ performance: c.sleepPerf }),
    recovery: ok({ score: c.recovery, band: recBand(c.recovery) }),
    strain: ok({ value: c.strain, band: strainBandOf(c.strain), targetLo: strainTarget[0], targetHi: strainTarget[1] }),
    healthMonitor: ok({ inRange: hm.length - flagged.length, total: hm.length, flagged: flagged.map((m) => ({ metric: m.key, label: m.label, status: m.status })) }),
    stress: ok({ current: cur, band: cur === null ? null : stressBandOf(cur), series: st }),
    tonightsSleep: ok({ needMin: c.needMin, recommendedBedtimeMs: dayStartMs(day) + (22 * 60 + 50) * MIN, wakeTimeMs: wake + 86_400_000 }),
    activities: activitiesFor(day),
    keyStats: ok({ hrvMs: c.hrv, restingHr: c.rhr, respRate: c.resp, calories: c.calories }),
    gaps: dayGaps(c),
  };
}

function sleep(day: Day): Result<unknown> {
  const c = core(day);
  if (!c.hasData) return noData(day);
  const segs = stageSegments(c);
  const m = minutesBy(segs);
  const asleep = m.light + m.deep + m.rem;
  const inBed = m.awake + asleep;
  const awakeAfter = segs.filter((s) => s.stage === "awake" && s.startMs > segs[0].endMs);
  const strainMin = c.strain > 10 ? Math.round((c.strain - 10) * 4) : 0;
  const debt = Math.round(15 + (hash(day) % 30));
  const naps = 0;
  const baseline = c.needMin - strainMin - debt;
  const wakeTime = c.wakeMs + 86_400_000;
  const eff = 0.9;
  const opt = (goal: "peak" | "perform" | "getBy", pct: number) => {
    const sleepMin = Math.round((c.needMin * pct) / 100);
    return { goal, goalPct: pct, sleepMin, bedtimeMs: wakeTime - Math.round(sleepMin / eff) * MIN };
  };
  const consistency = Array.from({ length: 14 }, (_, i) => {
    const d = addDays(day, -(13 - i));
    const cc = core(d);
    return { day: d, bedMs: cc.bedMs, wakeMs: cc.wakeMs };
  });
  return ok({
    day, sleepId: `sleep-${day}`, performance: c.sleepPerf,
    contributors: [
      { key: "hoursVsNeeded", label: "Hours vs. Needed", pct: c.hoursVsNeeded },
      { key: "consistency", label: "Sleep Consistency", pct: c.consistency },
      { key: "efficiency", label: "Sleep Efficiency", pct: c.efficiency },
      { key: "sleepStress", label: "Sleep Stress", pct: c.sleepStress },
    ],
    segments: segs,
    disturbanceTimesMs: awakeAfter.map((s) => s.startMs),
    stageMin: m,
    typicalPct: { awake: 5, light: 50, deep: 23, rem: 22 },
    restorative: { min: m.deep + m.rem, pctOfAsleep: round(((m.deep + m.rem) / asleep) * 100, 0) },
    stats: {
      timeInBedMin: inBed, asleepMin: asleep, efficiencyPct: round((asleep / inBed) * 100, 0),
      latencyMin: Math.round((segs[0].endMs - segs[0].startMs) / MIN) + 4, disturbances: awakeAfter.length,
      respRate: c.resp, bedtimeMs: c.bedMs, wakeMs: c.wakeMs,
    },
    need: { baselineMin: baseline, strainMin, debtMin: debt, napsMin: naps, totalMin: c.needMin, actualMin: asleep },
    planner: ok({ wakeTimeMs: wakeTime, typicalEfficiencyPct: 90, options: [opt("peak", 100), opt("perform", 85), opt("getBy", 75)] }),
    consistency, naps: [], gaps: dayGaps(c),
  });
}

function recovery(day: Day): Result<unknown> {
  const c = core(day);
  if (!c.hasData) return noData(day);
  // Calibrating example: the oldest days have too few nights behind them.
  if (daysBetween(day, today()) > FIRST_DATA_DAYS - 3) return { state: "calibrating", nightsRemaining: 4 - (FIRST_DATA_DAYS - daysBetween(day, today())), message: "Recovery needs a few nights to learn your baseline." };
  const base = { hrv: 62, rhr: 53, resp: 14.8, sleep: 85 };
  const mk = (key: "hrv" | "restingHr" | "respRate" | "sleepPerformance", label: string, value: number, unit: string, baseline: number, weight: number, lowerBetter: boolean) => {
    const deltaPct = round(((value - baseline) / baseline) * 100, 1);
    const good = lowerBetter ? deltaPct < -1 : deltaPct > 1;
    const bad = lowerBetter ? deltaPct > 1 : deltaPct < -1;
    return { key, label, value, unit, baseline, deltaPct, direction: good ? "good" : bad ? "bad" : "neutral", impactPts: round((lowerBetter ? -deltaPct : deltaPct) * weight * 0.6, 1) };
  };
  const last7 = Array.from({ length: 7 }, (_, i) => {
    const d = addDays(day, -(6 - i));
    const cc = core(d);
    return cc.hasData ? { day: d, score: cc.recovery, band: recBand(cc.recovery) } : { day: d, score: null, band: null };
  });
  const b = recBand(c.recovery);
  return ok({
    day, score: c.recovery, band: b,
    reading: b === "green" ? "Green: your body is primed for strain." : b === "yellow" ? "Yellow: a balanced day is a good fit." : "Red: prioritise rest and recovery today.",
    contributors: [
      mk("hrv", "HRV", c.hrv, "ms", base.hrv, 0.55 * 2, false),
      mk("restingHr", "Resting heart rate", c.rhr, "bpm", base.rhr, 0.2 * 2, true),
      mk("respRate", "Respiratory rate", c.resp, "rpm", base.resp, 0.05 * 2, true),
      mk("sleepPerformance", "Sleep performance", c.sleepPerf, "%", base.sleep, 0.15 * 2, false),
    ],
    last7,
  });
}

function strain(day: Day): Result<unknown> {
  const c = core(day);
  if (!c.hasData) return noData(day);
  const target = c.recovery >= 67 ? [14, 18] : c.recovery >= 34 ? [10, 14] : [0, 10];
  const hr = hrSeries(c);
  const acts = activitiesFor(day);
  const spans: Span[] = [
    { startMs: c.bedMs, endMs: c.wakeMs, kind: "sleep", label: "Sleep" },
    ...acts.map((a): Span => ({ startMs: a.startMs, endMs: a.endMs, kind: "activity", label: a.kind })),
    ...dayGaps(c).map((g): Span => ({ ...g, kind: "gap", label: g.reason })),
  ];
  return ok({
    day, value: c.strain, band: strainBandOf(c.strain), targetLo: target[0], targetHi: target[1],
    hr, zoneBounds: zoneBounds(c.rhr), maxHr: MAX_HR, zoneMin: c.zoneMin, spans, activities: acts,
    stats: { calories: c.calories, avgHr: c.avgHr, steps: c.steps, strengthMin: acts.filter((a) => a.kind === "Strength").reduce((s, a) => s + (a.endMs - a.startMs) / MIN, 0) },
    gaps: dayGaps(c),
  });
}

function activityDetail(id: string): ActivityDetail {
  const all = [...coreCacheActivities(), ...manualActivities];
  let a = all.find((x) => x.id === id);
  if (!a) {
    const day = id.split("-").slice(1, 4).join("-");
    a = core(day).activities.find((x) => x.id === id);
  }
  if (!a) throw new Error(`Unknown activity ${id}`);
  a = renamed.get(id) ?? a;
  const c = core(toDay(new Date(a.startMs)));
  const r = rng(hash(id));
  const ts: Ms[] = [];
  const v: (number | null)[] = [];
  const dur = Math.round((a.endMs - a.startMs) / MIN);
  for (let i = 0; i <= dur; i++) {
    ts.push(a.startMs + i * MIN);
    v.push(Math.round(clamp(a.avgHr + 16 * Math.sin((i / dur) * 9) + (r() - 0.5) * 10, c.rhr + 20, MAX_HR - 4)));
  }
  return { summary: a, hr: { ts, v }, zoneBounds: zoneBounds(c.rhr), maxHr: MAX_HR };
}
function* coreCacheActivities() { for (const c of coreCache.values()) yield* c.activities; }

function healthMetrics(c: DayCore): HealthMetric[] {
  const mk = (key: HealthMetricKey, label: string, value: number | null, unit: string, lo: number | null, hi: number | null, digits = 1): HealthMetric => {
    let status: StatusLevel = "unavailable";
    if (value !== null && lo !== null && hi !== null) {
      const span = hi - lo;
      status = value >= lo && value <= hi ? "ok" : value >= lo - span && value <= hi + span ? "warn" : "alert";
    }
    return { key, label, value: value === null ? null : round(value, digits), unit, rangeLo: lo, rangeHi: hi, status };
  };
  return [
    mk("respRate", "Respiratory rate", c.resp, "rpm", 14.0, 15.6),
    mk("spo2", "Blood oxygen", c.spo2, "%", 95, 99),
    mk("restingHr", "Resting heart rate", c.rhr, "bpm", 49, 57, 0),
    mk("hrv", "HRV", c.hrv, "ms", 50, 76, 0),
    mk("skinTemp", "Skin temperature", c.skinTemp, "°C", -0.5, 0.5),
  ];
}

const NOTES: Record<HealthMetricKey, string> = {
  respRate: "Illness, altitude, alcohol and heavy training can raise respiratory rate.",
  spo2: "Altitude, congestion and poor strap fit can lower blood oxygen readings.",
  restingHr: "Illness, alcohol, travel, dehydration and hard training can raise resting heart rate.",
  hrv: "Stress, alcohol, illness and poor sleep tend to lower HRV; good recovery raises it.",
  skinTemp: "Fever, a hot room or the menstrual cycle can shift skin temperature.",
};

function health(day: Day) {
  const c = core(day);
  if (!c.hasData) return { day, liveHr: null, metrics: noData(day), earlyWarning: null };
  const m = healthMetrics(c);
  const out = m.filter((x) => x.status !== "ok" && x.status !== "unavailable");
  return {
    day, liveHr: strap.state === "connected" ? lastHr : null, metrics: ok(m),
    earlyWarning: out.length >= 3 ? { message: "Several metrics moved together. Consider taking it easy.", metrics: out.map((x) => x.key) } : null,
  };
}

function metricDetail(metric: HealthMetricKey, day: Day): Result<unknown> {
  const c = core(day);
  if (!c.hasData) return noData(day);
  const cur = healthMetrics(c).find((m) => m.key === metric)!;
  const pick = (cc: DayCore) => metric === "respRate" ? cc.resp : metric === "spo2" ? cc.spo2 : metric === "restingHr" ? cc.rhr : metric === "hrv" ? cc.hrv : cc.skinTemp;
  const points = Array.from({ length: 30 }, (_, i) => {
    const d = addDays(day, -(29 - i));
    const cc = core(d);
    return { day: d, value: cc.hasData ? round(pick(cc), 1) : null, lo: cur.rangeLo, hi: cur.rangeHi };
  });
  return ok({ metric, label: cur.label, unit: cur.unit, current: cur.value, rangeLo: cur.rangeLo, rangeHi: cur.rangeHi, status: cur.status, points, note: NOTES[metric] });
}

function stress(day: Day): Result<unknown> {
  const c = core(day);
  if (!c.hasData) return noData(day);
  const s = stressSeries(c);
  const hours = { low: 0, medium: 0, high: 0 };
  s.v.forEach((x) => { if (x !== null) hours[stressBandOf(x)] += 5 / 60; });
  const cur = [...s.v].reverse().find((x) => x !== null) ?? null;
  return ok({
    day, current: cur, band: cur === null ? null : stressBandOf(cur), series: s,
    thresholds: { lowMax: 1, mediumMax: 2 },
    hoursInBand: { low: round(hours.low, 1), medium: round(hours.medium, 1), high: round(hours.high, 1) },
    spans: [{ startMs: c.bedMs, endMs: c.wakeMs, kind: "sleep" as const, label: "Sleep" }, ...c.activities.map((a): Span => ({ startMs: a.startMs, endMs: a.endMs, kind: "activity", label: a.kind }))],
    gaps: dayGaps(c),
  });
}

// ------------------------------------------------------------------ trends

const TREND_META: Record<TrendMetric, { label: string; unit: string; get: (c: DayCore) => number; band?: boolean; baseline?: [number, number] }> = {
  sleepPerformance: { label: "Sleep Performance", unit: "%", get: (c) => c.sleepPerf },
  hoursVsNeed: { label: "Hours vs. Need", unit: "%", get: (c) => c.hoursVsNeeded },
  timeInBed: { label: "Time in Bed", unit: "h", get: (c) => round((c.wakeMs - c.bedMs) / 3_600_000, 1) },
  consistency: { label: "Sleep Consistency", unit: "%", get: (c) => c.consistency },
  restorativePct: { label: "Restorative Sleep", unit: "%", get: (c) => 38 + (hash(c.day) % 14) },
  restorativeHours: { label: "Restorative Sleep", unit: "h", get: (c) => round(c.asleepMin / 60 * 0.42, 1) },
  efficiency: { label: "Sleep Efficiency", unit: "%", get: (c) => c.efficiency },
  sleepDebt: { label: "Sleep Debt", unit: "min", get: (c) => 15 + (hash(c.day) % 40) },
  dayStrain: { label: "Day Strain", unit: "", get: (c) => c.strain },
  strengthTime: { label: "Strength Activity Time", unit: "min", get: (c) => c.activities.filter((a) => a.kind === "Strength").reduce((s, a) => s + (a.endMs - a.startMs) / MIN, 0) },
  calories: { label: "Calories", unit: "kcal", get: (c) => c.calories },
  avgHr: { label: "Average HR", unit: "bpm", get: (c) => c.avgHr },
  steps: { label: "Steps", unit: "", get: (c) => c.steps },
  zones123: { label: "Zones 1-3", unit: "min", get: (c) => c.zoneMin[0] + c.zoneMin[1] + c.zoneMin[2] },
  zones45: { label: "Zones 4-5", unit: "min", get: (c) => c.zoneMin[3] + c.zoneMin[4] },
  vo2max: { label: "VO2 Max", unit: "ml/kg/min", get: (c) => round(46 + (hash(c.day.slice(0, 7)) % 30) / 10, 1) },
  recovery: { label: "Recovery", unit: "%", get: (c) => c.recovery, band: true },
  restingHr: { label: "Resting HR", unit: "bpm", get: (c) => c.rhr, baseline: [49, 57] },
  hrv: { label: "HRV", unit: "ms", get: (c) => c.hrv, baseline: [50, 76] },
  respRate: { label: "Respiratory Rate", unit: "rpm", get: (c) => c.resp, baseline: [14, 15.6] },
  stress: { label: "Stress", unit: "", get: (c) => round(0.9 + (hash(c.day) % 100) / 100, 2) },
};

const RANGE_DAYS: Record<TrendRange, number> = { "1W": 7, "1M": 30, "6M": 182, "1Y": 365 };

function trend(metric: TrendMetric, range: TrendRange): Result<unknown> {
  const meta = TREND_META[metric];
  const n = RANGE_DAYS[range];
  const end = today();
  const weekly = range === "6M" || range === "1Y";
  const days = Array.from({ length: n }, (_, i) => addDays(end, -(n - 1 - i)));
  const val = (d: Day) => { const c = core(d); return c.hasData ? meta.get(c) : null; };
  let points: { day: Day; value: number | null; band: Band | null }[];
  if (weekly) {
    points = [];
    for (let i = 0; i < n; i += 7) {
      const vs = days.slice(i, i + 7).map(val).filter((x): x is number => x !== null);
      const v = vs.length ? round(vs.reduce((a, b) => a + b, 0) / vs.length, 1) : null;
      points.push({ day: days[i], value: v, band: meta.band && v !== null ? recBand(v) : null });
    }
  } else {
    points = days.map((d) => { const v = val(d); return { day: d, value: v === null ? null : round(v, 1), band: meta.band && v !== null ? recBand(v) : null }; });
  }
  const real = points.filter((p): p is { day: Day; value: number; band: Band | null } => p.value !== null);
  if (!real.length) return { state: "noData", reason: "No data in this range yet.", gaps: [] };
  const avg = real.reduce((s, p) => s + p.value, 0) / real.length;
  const prev = days.slice(0, 0);
  void prev;
  const best = real.reduce((a, b) => (b.value > a.value ? b : a));
  const worst = real.reduce((a, b) => (b.value < a.value ? b : a));
  return ok({
    metric, label: meta.label, unit: meta.unit, range, granularity: weekly ? "weekly" : "daily", points,
    baseline: meta.baseline ? { lo: meta.baseline[0], hi: meta.baseline[1] } : null,
    summary: { avg: round(avg, 1), changePct: round(((hash(metric + range) % 120) - 60) / 10, 1), best: { day: best.day, value: best.value }, worst: { day: worst.day, value: worst.value } },
  });
}

// ------------------------------------------------------------------ journal

const LIBRARY: Omit<JournalLibraryItem, "selected">[] = [
  { id: "alcohol", label: "Had alcohol", category: "Nutrition" },
  { id: "caffeine_late", label: "Caffeine after 2 pm", category: "Nutrition" },
  { id: "late_meal", label: "Ate a late meal", category: "Nutrition" },
  { id: "screens_bed", label: "Screens in bed", category: "Sleep habits" },
  { id: "stretching", label: "Stretched or did mobility", category: "Wellness" },
  { id: "sauna", label: "Used a sauna", category: "Wellness" },
  { id: "magnesium", label: "Took magnesium", category: "Supplements" },
  { id: "travel", label: "Travelled", category: "Lifestyle" },
  { id: "illness", label: "Felt unwell", category: "Health" },
  { id: "meditation", label: "Meditated", category: "Wellness" },
];
let selected = new Set(["alcohol", "caffeine_late", "late_meal", "screens_bed", "stretching", "sauna"]);
const answers = new Map<string, boolean | null>();

function journal(day: Day) {
  return {
    day,
    questions: LIBRARY.filter((l) => selected.has(l.id)).map((l) => {
      const key = `${day}|${l.id}`;
      const answer = answers.has(key) ? answers.get(key)! : daysBetween(day, today()) > 0 ? hash(key) % 3 === 0 : null;
      return { id: l.id, label: l.label, category: l.category, answer };
    }),
  };
}

function insights(): Insight[] {
  return LIBRARY.filter((l) => selected.has(l.id)).map((l, i) => {
    const h = hash(l.id);
    const yes = 3 + (h % 20);
    const no = 12 + (h % 40);
    const good = ["stretching", "sauna", "magnesium", "meditation"].includes(l.id);
    const mag = 3 + (h % 9);
    const enough = yes >= 5 && no >= 5 && i !== 5;
    return {
      behavior: l.id, label: l.label, yesDays: i === 5 ? 3 : yes, noDays: i === 5 ? 7 : no,
      state: enough ? "ok" : "notEnough",
      recoveryDelta: enough ? (good ? mag : -mag) : null,
      sleepDelta: enough ? (good ? mag - 2 : -(mag - 1)) : null,
      hrvDelta: enough ? (good ? mag + 1 : -(mag + 2)) : null,
    };
  });
}

// ------------------------------------------------------------------ assessments

function assessmentList() {
  const t = today();
  return [
    { id: "w-latest", kind: "weekly", title: "Weekly assessment", periodStart: addDays(t, -7), periodEnd: addDays(t, -1), createdMs: dayStartMs(t) + 6 * 3_600_000 },
    { id: "m-latest", kind: "monthly", title: "Monthly assessment", periodStart: addDays(t, -30), periodEnd: addDays(t, -1), createdMs: dayStartMs(addDays(t, -2)) },
    { id: "y-latest", kind: "yearly", title: "Year in review", periodStart: addDays(t, -364), periodEnd: t, createdMs: dayStartMs(addDays(t, -20)) },
  ];
}
function assessment(id: string): Assessment {
  const item = assessmentList().find((a) => a.id === id) ?? assessmentList()[0];
  const n = daysBetween(item.periodStart, item.periodEnd) + 1;
  const days = Array.from({ length: n }, (_, i) => addDays(item.periodStart, i)).filter((d) => core(d).hasData);
  const cs = days.map(core);
  const avg = (f: (c: DayCore) => number) => (cs.length ? round(cs.reduce((s, c) => s + f(c), 0) / cs.length, 1) : null);
  return {
    ...(item as { id: string; title: string; periodStart: Day; periodEnd: Day }), kind: item.kind as Assessment["kind"],
    avgRecovery: avg((c) => c.recovery), avgStrain: avg((c) => c.strain), avgSleepPerformance: avg((c) => c.sleepPerf),
    hoursSlept: round(cs.reduce((s, c) => s + c.asleepMin, 0) / 60, 1), hoursNeeded: round(cs.reduce((s, c) => s + c.needMin, 0) / 60, 1),
    scatter: days.slice(0, -1).map((d, i) => ({ day: d, strain: cs[i].strain, nextRecovery: cs[i + 1].recovery })),
    consistency: cs.slice(-14).map((c) => ({ day: c.day, bedMs: c.bedMs, wakeMs: c.wakeMs })),
    ...(item.kind !== "weekly" ? {
            zonesByActivity: ["Run", "Strength", "Cycling", "Walk"].map((k) => { const acts = cs.flatMap((c) => c.activities.filter((a) => a.kind === k)); return { kind: k, sessions: acts.length, avgStrain: acts.length ? round(acts.reduce((t, a) => t + a.strain, 0) / acts.length, 1) : null, zoneMin: acts.reduce((z, a) => { a.zoneMin.forEach((m, i) => (z[i] += m)); return z; }, [0, 0, 0, 0, 0] as number[]) as [number, number, number, number, number] }; }),
      topJournalImpacts: insights().filter((i) => i.state === "ok").slice(0, 4),
      trainingResponse: { months: ["Jun", "Jul", "Aug", "Sep", "Oct", "Nov"], strain: [9.1, 10.2, 9.8, 11.0, 10.6, 11.4], recovery: [60, 63, 61, 66, 68, 67], restingHr: [57, 56, 55, 54, 54, 53] },
    } : {}),
    ...(item.kind === "yearly" ? {
      heat: (() => {
        const all = Array.from({ length: 365 }, (_, i) => addDays(today(), -(364 - i)));
        const f = (g: (c: DayCore) => number) => all.map((d) => ({ day: d, value: round(g(syntheticCore(d)), 1) }));
        return { recovery: f((c) => c.recovery), strain: f((c) => c.strain), sleep: f((c) => c.sleepPerf) };
      })(),
    } : {}),
  };
}
/** Older than the real mock window: still deterministic so the yearly heat map is full. */
function syntheticCore(d: Day): DayCore {
  const c = core(d);
  return c;
}

// ------------------------------------------------------------------ strap + sync

let strap: StrapStatus = {
  transport: "emulator", state: "disconnected", deviceName: null, batteryPct: 84, charging: false, onWrist: null,
  lastSyncMs: Date.now() - 42 * 60_000, lastHr: null, error: null,
};
let lastHr = 62;
let hrTimer: ReturnType<typeof setInterval> | null = null;
let settings: Settings = {
  profile: { age: profile.age, sex: "male", weightKg: 76, heightCm: 180 },
  transport: "emulator", emulatorAddress: "127.0.0.1:47800", autoSync: true, launchAtLogin: false,
};
const setStrap = (patch: Partial<StrapStatus>) => { strap = { ...strap, ...patch }; emit("strap://status", strap); };
const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));

function startStream() {
  stopStream();
  const r = rng(Date.now());
  hrTimer = setInterval(() => {
    if (strap.state !== "connected" && strap.state !== "syncing") return;
    if (strap.onWrist === false) return;
    lastHr = Math.round(clamp(lastHr + (r() - 0.5) * 4, 52, 96));
    emit("strap://hr", { ts: Date.now(), bpm: lastHr });
    setStrap({ lastHr });
    const bytes = Array.from({ length: 16 }, () => Math.floor(r() * 256).toString(16).padStart(2, "0")).join("");
    emit("strap://raw", { receivedAt: Date.now(), characteristic: "61080005-8d6d-82b8-614a-1c8cb0f8dcc6", hex: "aa01" + bytes });
  }, 1000);
}
function stopStream() { if (hrTimer) clearInterval(hrTimer); hrTimer = null; }

async function connect(transport: "ble" | "emulator") {
  setStrap({ transport, state: "scanning", error: null });
  await delay(500);
  if (transport === "ble") { setStrap({ state: "error", error: "Bluetooth is not available in browser preview mode." }); return; }
  setStrap({ state: "connecting" });
  await delay(600);
  setStrap({ state: "connected", deviceName: "Strap Emulator", onWrist: true });
  startStream();
}

async function sync() {
  if (strap.state !== "connected") throw new Error("Strap is not connected");
  setStrap({ state: "syncing" });
  const phases: [SyncProgress["phase"], number, string][] = [
    [ "requesting", 0.05, "Requesting history…" ], ["downloading", 0.25, "Downloading 23 h of data… 25%"],
    ["downloading", 0.64, "Downloading 23 h of data… 64%"], ["downloading", 0.95, "Downloading 23 h of data… 95%"],
    ["processing", 0.98, "Calculating scores…"], ["done", 1, "Synced"],
  ];
  for (const [phase, fraction, label] of phases) { emit("sync://progress", { phase, fraction, label }); await delay(600); }
  const now = Date.now();
  setStrap({ state: "connected", lastSyncMs: now });
  emit("data://updated", { days: [today(), addDays(today(), -1)] });
  emit("sync://progress", { phase: "idle", fraction: 0, label: "" });
}

// ------------------------------------------------------------------ dispatcher

type Args = Record<string, unknown>;
const s = (a: Args, k: string) => a[k] as string;

export async function handle(cmd: string, a: Args): Promise<unknown> {
  await delay(60 + (hash(cmd) % 80)); // feel like IPC
  switch (cmd) {
    case "get_home": return home(s(a, "day"));
    case "get_sleep": return sleep(s(a, "day"));
    case "get_recovery": return recovery(s(a, "day"));
    case "get_strain": return strain(s(a, "day"));
    case "get_activity": return activityDetail(s(a, "id"));
    case "add_activity": {
      const act: ActivitySummary = {
        id: `act-manual-${manualActivities.length}-${a.startMs}`, kind: s(a, "kind"), startMs: a.startMs as number, endMs: a.endMs as number,
        strain: 6.2, avgHr: 118, maxHr: 142, calories: 220, zoneMin: [10, 12, 6, 0, 0], manual: true,
      };
      manualActivities.push(act);
      emit("data://updated", { days: [toDay(new Date(act.startMs))] });
      return act;
    }
    case "rename_activity": {
      const cur = activityDetail(s(a, "id")).summary;
      const upd = { ...cur, kind: s(a, "kind") };
      renamed.set(upd.id, upd);
      emit("data://updated", { days: [toDay(new Date(upd.startMs))] });
      return upd;
    }
    case "get_health": return health(s(a, "day"));
    case "get_metric_detail": return metricDetail(a.metric as HealthMetricKey, s(a, "day"));
    case "get_stress": return stress(s(a, "day"));
    case "get_trend": return trend(a.metric as TrendMetric, a.range as TrendRange);
    case "get_journal": return journal(s(a, "day"));
    case "get_journal_library": return LIBRARY.map((l) => ({ ...l, selected: selected.has(l.id) }));
    case "set_journal_behaviors": selected = new Set(a.ids as string[]); return null;
    case "set_journal_answer": answers.set(`${s(a, "day")}|${s(a, "behavior")}`, a.answer as boolean | null); return null;
    case "get_insights": return insights();
    case "list_assessments": return assessmentList();
    case "get_assessment": return assessment(s(a, "id"));
    case "get_healthspan":
      return { state: "calibrating", nightsRemaining: 12, message: "Healthspan needs about 6 months of data. Showing a rough preview once 30 nights are in." } satisfies Result<never>;
    case "get_raw_day": { const c = core(s(a, "day")); return { day: c.day, hr: hrSeries(c), spans: [{ startMs: c.bedMs, endMs: c.wakeMs, kind: "sleep", label: "Sleep" }, ...c.activities.map((x): Span => ({ startMs: x.startMs, endMs: x.endMs, kind: "activity", label: x.kind })), ...dayGaps(c).map((g): Span => ({ ...g, kind: "gap", label: g.reason }))] }; }
    case "get_raw_night": {
      const c = core(s(a, "day"));
      if (!c.hasData) return noData(c.day);
      const r = rng(hash(c.day + "rr"));
      const full = hrSeries(c);
      const idx = full.ts.map((t, i) => (t >= c.bedMs && t < c.wakeMs ? i : -1)).filter((i) => i >= 0);
      const hr = { ts: idx.map((i) => full.ts[i]), v: idx.map((i) => full.v[i]) };
      const rr: TimeSeries = { ts: [], v: [] };
      for (let t = c.bedMs; t < c.wakeMs; t += 60_000 / 6) { // sample, not every beat
        const bpm = (hr.v[Math.min(hr.v.length - 1, Math.floor((t - c.bedMs) / MIN))] ?? 55) as number;
        rr.ts.push(t); rr.v.push(Math.round(60000 / bpm + (r() - 0.5) * 90));
      }
      return ok({ day: c.day, hr, rr });
    }
    case "get_raw_trends": {
      const n = RANGE_DAYS[a.range as TrendRange];
      return { range: a.range, days: Array.from({ length: n }, (_, i) => { const d = addDays(today(), -(n - 1 - i)); const c = core(d); return { day: d, restingHr: c.hasData ? c.rhr : null, hrv: c.hasData ? c.hrv : null }; }) };
    }
    case "export_csv": return `/Users/you/Downloads/strap-${s(a, "kind")}-${s(a, "from")}_${s(a, "to")}.csv`;
    case "get_strap_status": return strap;
    case "connect_strap": await connect(a.transport as "ble" | "emulator"); return null;
    case "disconnect_strap": stopStream(); setStrap({ state: "disconnected", lastHr: null, deviceName: null, onWrist: null }); return null;
    case "sync_now": await sync(); return null;
    case "rebuild_decoded": {
      for (const [phase, fraction, label] of [["processing", 0.3, "Re-decoding raw frames… 30%"], ["processing", 0.8, "Re-decoding raw frames… 80%"], ["done", 1, "Rebuilt"]] as [SyncProgress["phase"], number, string][]) { emit("sync://progress", { phase, fraction, label }); await delay(500); }
      emit("sync://progress", { phase: "idle", fraction: 0, label: "" });
      emit("data://updated", { days: [today()] });
      return null;
    }
    case "get_settings": return settings;
    case "set_settings": settings = a.settings as Settings; setStrap({ transport: settings.transport }); return settings;
    case "emulator_control": {
      const act = s(a, "action");
      if (act === "out_of_range") setStrap({ state: "disconnected", deviceName: null }), stopStream();
      else if (act === "in_range") await connect("emulator");
      else if (act === "take_off") setStrap({ onWrist: false });
      else if (act === "put_on") setStrap({ onWrist: true });
      else if (act === "advance_minutes") emit("data://updated", { days: [today()] });
      return null;
    }
    default: throw new Error(`mock: unknown command ${cmd}`);
  }
}

// Silence unused helper warnings for helpers kept for symmetry with the Rust side.
void parseDay;
