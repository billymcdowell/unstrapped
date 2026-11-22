import { useMemo, useState } from "react";
import * as api from "../api/client";
import type { Band, TrendData, TrendMetric, TrendRange } from "../api/types";
import { Card, DailyBars, EmptyState, ResultView, Segmented, StatTile } from "../components";
import { useApi, useColorScheme } from "../lib/hooks";
import { formatDayShort } from "../lib/date";
import { bandVar, cssVar, fmt } from "../lib/format";

type Group = "Sleep" | "Strain" | "Recovery" | "Stress";
const GROUPS: { group: Group; color: string; metrics: { id: TrendMetric; label: string }[] }[] = [
  { group: "Sleep", color: "--sleep", metrics: [
    { id: "sleepPerformance", label: "Sleep Performance" }, { id: "hoursVsNeed", label: "Hours vs Need" }, { id: "timeInBed", label: "Time in Bed" },
    { id: "consistency", label: "Consistency" }, { id: "restorativePct", label: "Restorative Sleep (%)" }, { id: "restorativeHours", label: "Restorative Sleep (hours)" },
    { id: "efficiency", label: "Efficiency" }, { id: "sleepDebt", label: "Sleep Debt" },
  ] },
  { group: "Strain", color: "--strain", metrics: [
    { id: "dayStrain", label: "Day Strain" }, { id: "strengthTime", label: "Strength Activity Time" }, { id: "calories", label: "Calories" },
    { id: "avgHr", label: "Average HR" }, { id: "steps", label: "Steps" }, { id: "zones123", label: "Zones 1–3" }, { id: "zones45", label: "Zones 4–5" }, { id: "vo2max", label: "VO₂ Max" },
  ] },
  { group: "Recovery", color: "--accent", metrics: [
    { id: "recovery", label: "Recovery" }, { id: "restingHr", label: "Resting HR" }, { id: "hrv", label: "HRV" }, { id: "respRate", label: "Respiratory Rate" },
  ] },
  { group: "Stress", color: "--stress", metrics: [{ id: "stress", label: "Stress" }] },
];
const colorFor = (m: TrendMetric) => GROUPS.find((g) => g.metrics.some((x) => x.id === m))!.color;
const RANGES: { value: TrendRange; label: string }[] = [{ value: "1W", label: "1W" }, { value: "1M", label: "1M" }, { value: "6M", label: "6M" }, { value: "1Y", label: "1Y" }];
const MINUS = "−";

export function Trends() {
  const [metric, setMetric] = useState<TrendMetric>("recovery");
  const [range, setRange] = useState<TrendRange>("1M");
  const { data, error } = useApi(() => api.getTrend(metric, range), [metric, range]);
  return (
    <div className="page">
      <div className="row wrap">
        <div>
          <h1 className="page-title">Trends</h1>
          <p className="page-sub">Pick a metric and a range.</p>
        </div>
        <span className="spacer" />
        <Segmented ariaLabel="Range" options={RANGES} value={range} onChange={setRange} />
      </div>
      <div className="split">
        <Card className="picker pad-sm">
          {GROUPS.map((g) => (
            <div key={g.group} className="picker-group">
              <div className="card-title" style={{ padding: "4px 8px" }}>{g.group}</div>
              {g.metrics.map((m) => (
                <button key={m.id} className="pick" aria-pressed={m.id === metric} onClick={() => setMetric(m.id)}>
                  <i style={{ background: `var(${g.color})` }} />{m.label}
                </button>
              ))}
            </div>
          ))}
        </Card>
        <div className="col" style={{ gap: "var(--s-4)", minWidth: 0 }}>
          {error && !data ? <EmptyState title="Couldn't load" detail={error} /> : <ResultView result={data}>{(d) => <TrendBody d={d} />}</ResultView>}
        </div>
      </div>
    </div>
  );
}

const digitsFor = (m: TrendMetric) => (m === "steps" || m === "calories" || m === "strengthTime" || m === "zones123" || m === "zones45" || m === "avgHr" || m === "sleepDebt" ? 0 : m === "stress" ? 2 : 1);

function TrendBody({ d }: { d: TrendData }) {
  const scheme = useColorScheme();
  const weekly = d.granularity === "weekly";
  const digits = digitsFor(d.metric);
  const unit = d.unit ? ` ${d.unit}` : "";
  const points = useMemo(() => d.points.map((p) => ({
    label: formatDayShort(p.day),
    value: p.value,
    color: d.metric === "recovery" && p.band ? bandVar(p.band as Band) : `var(${colorFor(d.metric)})`,
  })), [d]);
  const lineColor = useMemo(() => cssVar(colorFor(d.metric)), [d.metric, scheme]);
  const pct = d.unit === "%";
  const yMax = pct ? 100 : d.metric === "dayStrain" ? 21 : d.metric === "stress" ? 3 : undefined;
  const s = d.summary;
  const change = s?.changePct;
  const what = weekly ? "week" : "day";
  return (
    <>
      <Card title={`${d.label}${d.unit ? ` (${d.unit})` : ""}`}>
        <DailyBars points={points} line={weekly} color={lineColor} unit={unit} yMax={yMax} yMin={0} baseline={d.baseline} height={300} />
        <div className="legend" style={{ marginTop: 6 }}>
          <span>{weekly ? "Average per week" : "Daily values"}</span>
          {d.baseline && <span><i style={{ background: "var(--accent)", opacity: 0.3 }} />Your baseline</span>}
          {d.metric === "recovery" && !weekly && <><span><i style={{ background: "var(--rec-green)" }} />Green</span><span><i style={{ background: "var(--rec-yellow)" }} />Yellow</span><span><i style={{ background: "var(--rec-red)" }} />Red</span></>}
        </div>
      </Card>
      {s ? (
        <Card title="Summary">
          <div className="grid g4">
            <StatTile label="Average" value={fmt(s.avg, digits)} unit={d.unit} />
            <StatTile label="Vs previous period" value={change === null || change === undefined ? "—" : `${change > 0 ? "+" : change < 0 ? MINUS : ""}${Math.abs(change)}%`} sub={change === null || change === undefined ? "Not enough earlier data" : undefined} />
            <StatTile label={`Best ${what}`} value={fmt(s.best.value, digits)} unit={d.unit} sub={formatDayShort(s.best.day)} />
            <StatTile label={`Worst ${what}`} value={fmt(s.worst.value, digits)} unit={d.unit} sub={formatDayShort(s.worst.day)} />
          </div>
        </Card>
      ) : <EmptyState title="No summary yet" detail="Not enough data in this range." />}
    </>
  );
}
