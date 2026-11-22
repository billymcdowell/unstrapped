import { useMemo } from "react";
import * as api from "../api/client";
import type { Span, StressData } from "../api/types";
import { BreathingTimer, Card, ContributorBar, EmptyState, ResultView, TimeSeriesChart } from "../components";
import { useNav } from "../lib/app-context";
import { useApi, useColorScheme } from "../lib/hooks";
import { fmtTime } from "../lib/date";
import { cssVar, fmt, stressBandLabel } from "../lib/format";

const BAND_COLOR = { low: "var(--ok)", medium: "var(--warn)", high: "var(--alert)" } as const;

export function Stress() {
  const nav = useNav();
  const { data, error } = useApi(() => api.getStress(nav.day), [nav.day]);
  return (
    <div className="page">
      <div>
        <h1 className="page-title">Stress Monitor</h1>
        <p className="page-sub">Scale 0–3, estimated from heart rate and HRV against your baseline.</p>
      </div>
      {error && !data ? <EmptyState title="Couldn't load" detail={error} /> : <ResultView result={data}>{(d) => <StressBody d={d} />}</ResultView>}
      <Card title="Guided breathing"><BreathingTimer /></Card>
    </div>
  );
}

function StressBody({ d }: { d: StressData }) {
  const scheme = useColorScheme();
  const bands = useMemo(() => {
    const a = (v: string) => `${cssVar(v)}26`;
    return [
      { from: 0, to: d.thresholds.lowMax, color: a("--ok") },
      { from: d.thresholds.lowMax, to: d.thresholds.mediumMax, color: a("--warn") },
      { from: d.thresholds.mediumMax, to: 3, color: a("--alert") },
    ];
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [d.thresholds, scheme]);
  const spans: Span[] = useMemo(() => [...d.spans, ...d.gaps.map((g): Span => ({ ...g, kind: "gap", label: g.reason }))], [d.spans, d.gaps]);
  const total = d.hoursInBand.low + d.hoursInBand.medium + d.hoursInBand.high;
  const hasSeries = d.series.ts.length > 1;
  return (
    <>
      <section className={`card tone ${d.band === "high" ? "tone-coral" : d.band === "medium" ? "tone-yellow" : "tone-rose"} hero`}>
        <div className="stat">
          <span className="score-label">Current stress</span>
          <span className="hero-value num">{fmt(d.current, 1)}<span className="score-unit">/ 3</span></span>
        </div>
        <div className="col" style={{ gap: 8 }}>
          <span className="pill" style={{ alignSelf: "flex-start" }}>{d.band ? stressBandLabel[d.band] : "No current reading"}</span>
          <p className="muted">Low 0–{d.thresholds.lowMax} · Medium {d.thresholds.lowMax}–{d.thresholds.mediumMax} · High {d.thresholds.mediumMax}–3</p>
        </div>
      </section>

      <Card title="Since you woke up">
        {hasSeries ? (
          <>
            <TimeSeriesChart
              series={d.series} height={240} color="var(--stress)" bands={bands} spans={spans} yRange={[0, 3]}
              ariaLabel="Stress from waking until now"
            />
            <div className="legend" style={{ marginTop: 6 }}>
              <span><i style={{ background: "var(--ok)", opacity: 0.5 }} />Low</span>
              <span><i style={{ background: "var(--warn)", opacity: 0.5 }} />Medium</span>
              <span><i style={{ background: "var(--alert)", opacity: 0.5 }} />High</span>
              <span><i style={{ background: "var(--span-sleep)" }} />Sleep</span>
              <span><i style={{ background: "var(--span-activity)" }} />Activity</span>
              <span><i style={{ background: "var(--span-gap)" }} />Strap off</span>
            </div>
            {d.gaps.filter((g) => g.endMs - g.startMs < 23 * 3_600_000).map((g, i) => <div key={i} className="faint num">{g.reason} {fmtTime(g.startMs)}–{fmtTime(g.endMs)}</div>)}
          </>
        ) : <EmptyState title="Not enough readings yet" detail="The graph appears after a few readings since waking." gaps={d.gaps} />}
      </Card>

      <Card title="Time in each band">
        <div className="col" style={{ gap: 14 }}>
          {(["low", "medium", "high"] as const).map((b) => (
            <ContributorBar key={b} label={stressBandLabel[b]} value={d.hoursInBand[b]} max={Math.max(total, 0.1)} valueText={`${fmt(d.hoursInBand[b], 1)} h`} color={BAND_COLOR[b]} />
          ))}
        </div>
      </Card>
    </>
  );
}
