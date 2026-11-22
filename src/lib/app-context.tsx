import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import type { DataUpdatedEvent, Day, StrapStatus, SyncProgress } from "../api/types";
import * as api from "../api/client";
import { today } from "./date";

export type ScreenId =
  | "home" | "sleep" | "recovery" | "strain" | "health" | "stress" | "trends"
  | "journal" | "assessments" | "healthspan" | "data" | "strap";

export interface Nav {
  screen: ScreenId;
  go: (s: ScreenId) => void;
  day: Day;
  setDay: (d: Day) => void;
  activityId: string | null;
  /** Opens the Strain screen on an activity detail view. */
  openActivity: (id: string | null) => void;
  /** Bumped whenever `data://updated` touches the current day (or any day for non-day screens). Include in effect deps to refetch. */
  refreshKey: number;
  /** Force a refetch of the current screen (e.g. after a local mutation). */
  refresh: () => void;
}

const NavCtx = createContext<Nav | null>(null);
export const useNav = (): Nav => {
  const v = useContext(NavCtx);
  if (!v) throw new Error("useNav outside provider");
  return v;
};

export interface RawLine { receivedAt: number; characteristic: string; hex: string }

export interface StrapCtx {
  status: StrapStatus | null;
  progress: SyncProgress | null;
  liveHr: { ts: number; bpm: number } | null;
  rawLog: RawLine[];
  clearRawLog: () => void;
}
const StrapCtxI = createContext<StrapCtx | null>(null);
export const useStrap = (): StrapCtx => {
  const v = useContext(StrapCtxI);
  if (!v) throw new Error("useStrap outside provider");
  return v;
};

const RAW_LOG_MAX = 400;

export function AppProviders({ children }: { children: ReactNode }) {
  const [screen, setScreen] = useState<ScreenId>("home");
  const [day, setDay] = useState<Day>(today());
  const [activityId, setActivityId] = useState<string | null>(null);
  const [refreshKey, setRefreshKey] = useState(0);

  const [status, setStatus] = useState<StrapStatus | null>(null);
  const [progress, setProgress] = useState<SyncProgress | null>(null);
  const [liveHr, setLiveHr] = useState<StrapCtx["liveHr"]>(null);
  const [rawLog, setRawLog] = useState<RawLine[]>([]);

  useEffect(() => {
    let cancelled = false;
    const offs: api.Unlisten[] = [];
    api.getStrapStatus().then((s) => !cancelled && setStatus(s)).catch(() => {});
    const reg = async () => {
      const list = [
        await api.onStrapStatus(setStatus),
        await api.onSyncProgress((p) => setProgress(p.phase === "idle" ? null : p)),
        await api.onStrapHr((h) => setLiveHr(h)),
        await api.onStrapRaw((f) => setRawLog((l) => [...l.slice(-(RAW_LOG_MAX - 1)), f])),
        await api.onDataUpdated((_d: DataUpdatedEvent) => setRefreshKey((k) => k + 1)),
      ];
      if (cancelled) list.forEach((f) => f()); else offs.push(...list);
    };
    reg();
    return () => { cancelled = true; offs.forEach((f) => f()); };
  }, []);

  const nav = useMemo<Nav>(() => ({
    screen, day, setDay, activityId, refreshKey,
    refresh: () => setRefreshKey((k) => k + 1),
    go: (s) => { setScreen(s); setActivityId(null); },
    openActivity: (id) => { setActivityId(id); if (id) setScreen("strain"); },
  }), [screen, day, activityId, refreshKey]);

  const strap = useMemo<StrapCtx>(() => ({ status, progress, liveHr, rawLog, clearRawLog: () => setRawLog([]) }), [status, progress, liveHr, rawLog]);

  return (
    <NavCtx.Provider value={nav}>
      <StrapCtxI.Provider value={strap}>{children}</StrapCtxI.Provider>
    </NavCtx.Provider>
  );
}
