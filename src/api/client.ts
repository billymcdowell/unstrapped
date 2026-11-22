/**
 * Typed wrappers over Tauri commands and events. One function per command.
 * JS arg keys are camelCase; Tauri maps them to the snake_case Rust parameters
 * (documented per command in docs/API_CONTRACT.md). Outside Tauri every call is
 * answered by ./mock.ts so `npm run dev` works in a normal browser.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { isTauri } from "./runtime";
import * as mock from "./mock";
import type {
  ActivityDetail, ActivitySummary, Assessment, AssessmentListItem, DataUpdatedEvent, Day, EmulatorAction,
  ExportKind, HealthData, Healthspan, HomeData, HrEvent, Insight, JournalDay, JournalLibraryItem, MetricDetail,
  RawDay, RawFrameEvent, RawNight, RawTrends, RecoveryData, Result, Settings, SleepData, StrainData,
  StrapStatus, StressData, SyncProgress, TrendData, TrendMetric, TrendRange, Transport, HealthMetricKey,
} from "./types";

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) return mock.handle(cmd, args ?? {}) as Promise<T>;
  return invoke<T>(cmd, args);
}

// ---- screens
export const getHome = (day: Day) => call<HomeData>("get_home", { day });
export const getSleep = (day: Day) => call<Result<SleepData>>("get_sleep", { day });
export const getRecovery = (day: Day) => call<Result<RecoveryData>>("get_recovery", { day });
export const getStrain = (day: Day) => call<Result<StrainData>>("get_strain", { day });
export const getActivity = (id: string) => call<ActivityDetail>("get_activity", { id });
export const addActivity = (kind: string, startMs: number, endMs: number) =>
  call<ActivitySummary>("add_activity", { kind, startMs, endMs });
export const renameActivity = (id: string, kind: string) => call<ActivitySummary>("rename_activity", { id, kind });
export const getHealth = (day: Day) => call<HealthData>("get_health", { day });
export const getMetricDetail = (metric: HealthMetricKey, day: Day) =>
  call<Result<MetricDetail>>("get_metric_detail", { metric, day });
export const getStress = (day: Day) => call<Result<StressData>>("get_stress", { day });
export const getTrend = (metric: TrendMetric, range: TrendRange) =>
  call<Result<TrendData>>("get_trend", { metric, range });
export const getJournal = (day: Day) => call<JournalDay>("get_journal", { day });
export const getJournalLibrary = () => call<JournalLibraryItem[]>("get_journal_library");
export const setJournalBehaviors = (ids: string[]) => call<void>("set_journal_behaviors", { ids });
export const setJournalAnswer = (day: Day, behavior: string, answer: boolean | null) =>
  call<void>("set_journal_answer", { day, behavior, answer });
export const getInsights = () => call<Insight[]>("get_insights");
export const listAssessments = () => call<AssessmentListItem[]>("list_assessments");
export const getAssessment = (id: string) => call<Assessment>("get_assessment", { id });
export const getHealthspan = () => call<Result<Healthspan>>("get_healthspan");

// ---- raw dashboard
export const getRawDay = (day: Day) => call<RawDay>("get_raw_day", { day });
export const getRawNight = (day: Day) => call<Result<RawNight>>("get_raw_night", { day });
export const getRawTrends = (range: TrendRange) => call<RawTrends>("get_raw_trends", { range });
/** Resolves to the absolute path of the saved file. */
export const exportCsv = (kind: ExportKind, from: Day, to: Day) =>
  call<string>("export_csv", { kind, from, to });

// ---- strap
export const getStrapStatus = () => call<StrapStatus>("get_strap_status");
export const connectStrap = (transport: Transport) => call<void>("connect_strap", { transport });
export const disconnectStrap = () => call<void>("disconnect_strap");
export const syncNow = () => call<void>("sync_now");
export const rebuildDecoded = () => call<void>("rebuild_decoded");
export const getSettings = () => call<Settings>("get_settings");
export const setSettings = (settings: Settings) => call<Settings>("set_settings", { settings });
export const emulatorControl = (action: EmulatorAction, value?: number) =>
  call<void>("emulator_control", { action, value: value ?? null });

// ---- events
export type Unlisten = () => void;

async function sub<T>(event: string, cb: (p: T) => void): Promise<Unlisten> {
  if (!isTauri()) return mock.subscribe(event, cb as (p: unknown) => void);
  return listen<T>(event, (e) => cb(e.payload));
}

export const onStrapStatus = (cb: (s: StrapStatus) => void) => sub<StrapStatus>("strap://status", cb);
export const onStrapRaw = (cb: (f: RawFrameEvent) => void) => sub<RawFrameEvent>("strap://raw", cb);
export const onStrapHr = (cb: (h: HrEvent) => void) => sub<HrEvent>("strap://hr", cb);
export const onSyncProgress = (cb: (p: SyncProgress) => void) => sub<SyncProgress>("sync://progress", cb);
export const onDataUpdated = (cb: (d: DataUpdatedEvent) => void) => sub<DataUpdatedEvent>("data://updated", cb);
