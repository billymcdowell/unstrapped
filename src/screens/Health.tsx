import { useState } from "react";
import * as api from "../api/client";
import type { HealthData, HealthMetric, HealthMetricKey, MetricDetail } from "../api/types";
import { Card, EmptyState, RangeBandChart, ResultView, StatusIcon } from "../components";
import { useNav, useStrap } from "../lib/app-context";
import { useApi } from "../lib/hooks";
import { formatDayShort } from "../lib/date";
import { fmt } from "../lib/format";

const DIGITS: Record<HealthMetricKey, number> = { respRate: 1, spo2: 0, restingHr: 0, hrv: 0, skinTemp: 1 };
const MINUS = "−";
/** Skin temperature is a deviation from your baseline, so it shows a sign. */
const val = (k: HealthMetricKey, v: number | null) =>
  v === null ? "—" : k === "skinTemp" ? `${v > 0 ? "+" : v < 0 ? MINUS : ""}${Math.abs(v).toFixed(1)}` : fmt(v, DIGITS[k]);
const typical = (m: { key: HealthMetricKey; rangeLo: number | null; rangeHi: number | null }) =>
  m.rangeLo === null || m.rangeHi === null ? "Typical range still forming" : `Typical ${val(m.key, m.rangeLo)}${m.key === "skinTemp" ? " to " : "–"}${val(m.key, m.rangeHi)}`;
const STATUS_TEXT = { ok: "Within your typical range", warn: "Slightly outside your typical range", alert: "Well outside your typical range", unavailable: "No reading last night" } as const;

export function Health() {
  const [open, setOpen] = useState<HealthMetricKey | null>(null);
  if (open) return <MetricView metric={open} onBack={() => setOpen(null)} />;
  return <HealthMain onOpen={setOpen} />;
}

function LiveHr({ fallback }: { fallback: number | null }) {
  const { status, liveHr } = useStrap();
  const connected = status?.state === "connected" || status?.state === "syncing";
  const bpm = connected ? (liveHr?.bpm ?? status?.lastHr ?? fallback) : null;
  return (
    <Card title="Heart rate now">
      {bpm !== null ? (
        <div className="row">
          <span className="heartbeat" aria-hidden="true">♥</span>
          <span className="live-hr num">{bpm}</span>
          <span className="muted">bpm</span>
          <span className="pill">Live</span>
        </div>
      ) : (
        <div className="col" style={{ gap: 2 }}>
          <span style={{ fontSize: 20, fontWeight: 650 }}>Live when connected</span>
          <span className="muted">Connect your strap on the Strap screen to see your heart rate update here.</span>
        </div>
      )}
    </Card>
  );
}

function HealthMain({ onOpen }: { onOpen: (k: HealthMetricKey) => void }) {
  const nav = useNav();
  const { data, error } = useApi(() => api.getHealth(nav.day), [nav.day]);
  if (!data) return <div className="page"><h1 className="page-title">Health Monitor</h1>{error ? <EmptyState title="Couldn't load" detail={error} /> : <div className="empty faint">Loading…</div>}</div>;
  return (
    <div className="page">
      <div>
        <h1 className="page-title">Health Monitor</h1>
        <p className="page-sub">Last night's readings against your own typical range.</p>
      </div>
      <LiveHr fallback={data.liveHr} />
      {data.earlyWarning && <Warning w={data.earlyWarning} />}
      <Card title="Last night">
        <ResultView result={data.metrics}>{(m) => <MetricRows metrics={m} onOpen={onOpen} />}</ResultView>
      </Card>
      <p className="faint">Estimates from a wrist sensor, not medical measurements.</p>
    </div>
  );
}

function Warning({ w }: { w: NonNullable<HealthData["earlyWarning"]> }) {
  return (
    <div className="banner warn" role="alert">
      <StatusIcon level="warn" size={22} />
      <div className="col" style={{ gap: 2 }}>
        <b>Early warning</b>
        <span>{w.message}</span>
      </div>
    </div>
  );
}

function MetricRows({ metrics, onOpen }: { metrics: HealthMetric[]; onOpen: (k: HealthMetricKey) => void }) {
  // Blood oxygen row is hidden when the strap did not report it.
  const rows = metrics.filter((m) => !(m.key === "spo2" && (m.status === "unavailable" || m.value === null)));
  const inRange = rows.filter((m) => m.status === "ok").length;
  return (
    <>
      <p className="muted" style={{ marginBottom: 6 }}>{inRange} of {rows.length} within range</p>
      <div className="list">
        {rows.map((m) => (
          <button key={m.key} className="list-row metric-row" onClick={() => onOpen(m.key)}>
            <span className="metric-name">{m.label}</span>
            <span className="spacer" />
            <span className="muted num">{typical(m)}</span>
            <span className="metric-val num"><b>{val(m.key, m.value)}</b> <span className="faint">{m.unit}</span></span>
            <StatusIcon level={m.status} />
            <span className="faint" aria-hidden="true">›</span>
          </button>
        ))}
      </div>
    </>
  );
}

function MetricView({ metric, onBack }: { metric: HealthMetricKey; onBack: () => void }) {
  const nav = useNav();
  const { data, error } = useApi(() => api.getMetricDetail(metric, nav.day), [metric, nav.day]);
  return (
    <div className="page">
      <div className="row"><button className="btn sm" onClick={onBack}>◀ Back to Health Monitor</button></div>
      {error && !data ? <EmptyState title="Couldn't load" detail={error} /> : (
        <ResultView result={data}>{(d) => <MetricBody d={d} />}</ResultView>
      )}
    </div>
  );
}

function MetricBody({ d }: { d: MetricDetail }) {
  const digits = DIGITS[d.metric];
  const outside = d.points.filter((p) => p.value !== null && p.lo !== null && p.hi !== null && (p.value < p.lo || p.value > p.hi)).length;
  return (
    <>
      <div className="row wrap">
        <h1 className="page-title">{d.label}</h1>
        <StatusIcon level={d.status} />
        <span className="muted">{STATUS_TEXT[d.status]}</span>
      </div>
      <Card>
        <div className="row wrap" style={{ gap: 32 }}>
          <div className="stat"><span className="stat-label">Last night</span><span className="stat-value num" style={{ fontSize: 34 }}>{val(d.metric, d.current)}<span className="stat-unit">{d.unit}</span></span></div>
          <div className="stat"><span className="stat-label">Your typical range</span><span className="stat-value num">{typical({ key: d.metric, rangeLo: d.rangeLo, rangeHi: d.rangeHi }).replace("Typical ", "")}<span className="stat-unit">{d.unit}</span></span></div>
        </div>
      </Card>
      <Card title="Last 30 days">
        <RangeBandChart points={d.points.map((p) => ({ label: formatDayShort(p.day), value: p.value, lo: p.lo, hi: p.hi }))} unit={d.unit} digits={digits} height={270} />
        <div className="legend" style={{ marginTop: 6 }}>
          <span><i style={{ background: "var(--accent)", opacity: 0.3 }} />Typical range</span>
          <span><i style={{ background: "var(--rec-red)", borderRadius: "50%" }} />Outside range ({outside} night{outside === 1 ? "" : "s"})</span>
        </div>
      </Card>
      <Card title="What can shift this">
        <p>{d.note}</p>
      </Card>
    </>
  );
}
