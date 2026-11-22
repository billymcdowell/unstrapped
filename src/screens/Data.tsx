import { useMemo, useState } from "react";
import * as api from "../api/client";
import type { Day, ExportKind, TrendRange } from "../api/types";
import { Card, DailyBars, EmptyState, ResultView, Segmented, TimeSeriesChart } from "../components";
import { useNav } from "../lib/app-context";
import { useApi } from "../lib/hooks";
import { addDays, dayEndMs, dayStartMs, formatDayShort, today } from "../lib/date";

export function Data() {
  return (
    <div className="page">
      <div>
        <h1 className="page-title">Data</h1>
        <p className="page-sub">Raw heart rate and beat-to-beat data straight from your strap.</p>
      </div>
      <TodayCard />
      <NightCard />
      <TrendsCard />
      <ExportCard />
    </div>
  );
}

function TodayCard() {
  const nav = useNav();
  const { data, error } = useApi(() => api.getRawDay(nav.day), [nav.day]);
  const hasHr = !!data && data.hr.v.some((v) => v !== null);
  return (
    <Card title="Today: heart rate over 24 hours">
      {error && !data ? <EmptyState title="Couldn't load" detail={error} /> : !data ? <div className="empty faint">Loading…</div> : !hasHr ? (
        <EmptyState title="No heart-rate data for this day" detail="Sync your strap to fill this in." />
      ) : (
        <>
          <TimeSeriesChart series={data.hr} height={240} color="var(--rec-red)" spans={data.spans} xRange={[dayStartMs(nav.day), dayEndMs(nav.day)]} ariaLabel="Heart rate over 24 hours" />
          <div className="legend" style={{ marginTop: 6 }}>
            <span><i style={{ background: "var(--span-sleep)" }} />Sleep</span>
            <span><i style={{ background: "var(--span-activity)" }} />Activity</span>
            <span><i style={{ background: "var(--span-gap)" }} />Strap off</span>
          </div>
        </>
      )}
    </Card>
  );
}

function NightCard() {
  const nav = useNav();
  const { data, error } = useApi(() => api.getRawNight(nav.day), [nav.day]);
  return (
    <Card title="Night: heart rate and R-R intervals">
      {error && !data ? <EmptyState title="Couldn't load" detail={error} /> : (
        <ResultView result={data}>
          {(d) => (
            <div className="col" style={{ gap: 14 }}>
              <div>
                <div className="stat-label" style={{ marginBottom: 4 }}>Heart rate (bpm)</div>
                <TimeSeriesChart series={d.hr} height={170} color="var(--rec-red)" ariaLabel="Overnight heart rate" />
              </div>
              <div>
                <div className="stat-label" style={{ marginBottom: 4 }}>R-R interval (ms between beats). The basis of HRV.</div>
                {d.rr.ts.length ? <TimeSeriesChart series={d.rr} height={170} color="var(--sleep)" ariaLabel="R-R intervals overnight" /> : <EmptyState title="No beat-to-beat data for this night" />}
              </div>
            </div>
          )}
        </ResultView>
      )}
    </Card>
  );
}

function TrendsCard() {
  const [range, setRange] = useState<TrendRange>("1M");
  const { data, error } = useApi(() => api.getRawTrends(range), [range]);
  const { rhr, hrv } = useMemo(() => ({
    rhr: (data?.days ?? []).map((d) => ({ label: formatDayShort(d.day), value: d.restingHr })),
    hrv: (data?.days ?? []).map((d) => ({ label: formatDayShort(d.day), value: d.hrv })),
  }), [data]);
  const any = !!data && data.days.some((d) => d.restingHr !== null || d.hrv !== null);
  return (
    <Card title="Trends: resting HR and HRV" action={undefined}>
      <div style={{ marginBottom: 10 }}>
        <Segmented ariaLabel="Range" value={range} onChange={setRange} options={[{ value: "1W", label: "1W" }, { value: "1M", label: "1M" }, { value: "6M", label: "6M" }, { value: "1Y", label: "1Y" }]} />
      </div>
      {error && !data ? <EmptyState title="Couldn't load" detail={error} /> : !data ? <div className="empty faint">Loading…</div> : !any ? (
        <EmptyState title="No resting HR or HRV in this range" />
      ) : (
        <div className="grid g2">
          <div><div className="stat-label">Resting HR (bpm)</div><DailyBars points={rhr} line color="#e5484d" unit=" bpm" height={200} /></div>
          <div><div className="stat-label">HRV (ms)</div><DailyBars points={hrv} line color="#7a6ae6" unit=" ms" height={200} /></div>
        </div>
      )}
    </Card>
  );
}

const KINDS: { kind: ExportKind; label: string }[] = [
  { kind: "heart_rate", label: "Heart rate" }, { kind: "rr_intervals", label: "R-R intervals" }, { kind: "daily_summary", label: "Daily summary" },
  { kind: "sleeps", label: "Sleeps" }, { kind: "activities", label: "Activities" }, { kind: "stress", label: "Stress" }, { kind: "raw_frames", label: "Raw frames" },
];

function ExportCard() {
  const [from, setFrom] = useState<Day>(addDays(today(), -29));
  const [to, setTo] = useState<Day>(today());
  const [busy, setBusy] = useState<ExportKind | null>(null);
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const run = async (kind: ExportKind) => {
    setBusy(kind); setMsg(null);
    try { setMsg({ ok: true, text: `Saved ${await api.exportCsv(kind, from, to)}` }); }
    catch (e) { setMsg({ ok: false, text: `Export failed: ${String((e as Error)?.message ?? e)}` }); }
    finally { setBusy(null); }
  };
  const bad = !from || !to || from > to;
  return (
    <Card title="Export to CSV">
      <div className="row wrap" style={{ marginBottom: 12 }}>
        <div className="field"><label htmlFor="exp-from">From</label><input id="exp-from" type="date" value={from} max={to} onChange={(e) => setFrom(e.target.value)} /></div>
        <div className="field"><label htmlFor="exp-to">To</label><input id="exp-to" type="date" value={to} min={from} onChange={(e) => setTo(e.target.value)} /></div>
      </div>
      <div className="row wrap" style={{ gap: 8 }}>
        {KINDS.map((k) => <button key={k.kind} className="btn" disabled={bad || busy !== null} onClick={() => run(k.kind)}>{busy === k.kind ? "Exporting…" : k.label}</button>)}
      </div>
      {bad && <p className="faint" style={{ marginTop: 8 }}>Choose a start day that is not after the end day.</p>}
      {msg && <p className={msg.ok ? "muted" : "error-text"} style={{ marginTop: 10, userSelect: "text" }} role="status">{msg.text}</p>}
    </Card>
  );
}
