import * as api from "../api/client";
import type { SleepData } from "../api/types";
import { Card, ContributorBar, Dial, EmptyState, Hypnogram, ResultView, StackedBar, StatTile, TimeRangeBars } from "../components";
import { useNav } from "../lib/app-context";
import { useApi } from "../lib/hooks";
import { formatDayShort, fmtDuration, fmtHM, fmtTime } from "../lib/date";
import { fmt } from "../lib/format";

const STAGES = [
  { key: "awake", label: "Awake", color: "var(--stage-awake)" },
  { key: "light", label: "Light", color: "var(--stage-light)" },
  { key: "deep", label: "Deep (SWS)", color: "var(--stage-deep)" },
  { key: "rem", label: "REM", color: "var(--stage-rem)" },
] as const;


export function Sleep() {
  const nav = useNav();
  const { data, error } = useApi(() => api.getSleep(nav.day), [nav.day]);
  if (error && !data) return <div className="page"><EmptyState title="Couldn't load" detail={error} /></div>;
  return (
    <div className="page">
      <h1 className="page-title">Sleep</h1>
      <ResultView result={data}>{(d) => <SleepBody d={d} />}</ResultView>
    </div>
  );
}

function SleepBody({ d }: { d: SleepData }) {
  const nav = useNav();
  const asleep = d.stats.asleepMin;
  const pct = (m: number) => (asleep + d.stageMin.awake > 0 ? (m / (asleep + d.stageMin.awake)) * 100 : 0);
  const stageRows = [
    { name: "You", parts: STAGES.map((s) => ({ label: s.label, value: d.stageMin[s.key], color: s.color })) },
    { name: "Typical", faint: true, parts: STAGES.map((s) => ({ label: s.label, value: d.typicalPct[s.key], color: s.color })) },
  ];
  const needParts = [
    { label: "Baseline", value: d.need.baselineMin, color: "var(--sleep)" },
    { label: "Strain", value: d.need.strainMin, color: "var(--strain)" },
    { label: "Debt", value: d.need.debtMin, color: "var(--stage-awake)" },
  ];
  return (
    <>
      <div className="grid g-main">
        <section className="card tone tone-dusk hero">
          <div className="col" style={{ alignItems: "center", gap: 4 }}>
            <Dial bare label="Sleep performance" color="var(--text)" value={d.performance} size={170} />
          </div>
          <div className="col" style={{ gap: 14 }}>
            <div className="row" style={{ alignItems: "baseline" }}>
              <span className="hero-value num" style={{ fontSize: 72 }}>{Math.round(d.performance)}<span className="score-unit">%</span></span>
              <span className="muted">Sleep performance</span>
            </div>
            {d.contributors.map((c) => <ContributorBar key={c.key} label={c.label} value={c.pct ?? 0} valueText={c.pct === null ? "—" : undefined} color="var(--text)" />)}
          </div>
        </section>
        <Card title="Restorative sleep">
          <StatTile label="Deep + REM" value={fmtDuration(d.restorative.min)} sub={`${fmt(d.restorative.pctOfAsleep)}% of time asleep · 40–50% is optimal`} />
        </Card>
      </div>

      <Card title="Night">
        <Hypnogram segments={d.segments} disturbancesMs={d.disturbanceTimesMs} />
        {d.gaps.length > 0 && d.gaps.map((g, i) => <div key={i} className="faint num">{g.reason} {fmtTime(g.startMs)}–{fmtTime(g.endMs)}</div>)}
      </Card>

      <div className="grid g2">
        <Card title="Stages">
          <StackedBar rows={stageRows} normalize formatValue={(v) => `${Math.round(v)}%`} height={100} />
          <table style={{ width: "100%", marginTop: 8, borderCollapse: "collapse" }}>
            <tbody>
              {STAGES.map((s) => (
                <tr key={s.key} className="num">
                  <td style={{ padding: "4px 0" }}><span className="legend"><span><i style={{ background: s.color }} />{s.label}</span></span></td>
                  <td style={{ textAlign: "right" }}>{fmtDuration(d.stageMin[s.key])}</td>
                  <td style={{ textAlign: "right", width: 60 }}>{Math.round(pct(d.stageMin[s.key]))}%</td>
                  <td className="faint" style={{ textAlign: "right", width: 70 }}>typ. {d.typicalPct[s.key]}%</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Card>
        <Card title="Night stats">
          <div className="grid g3" style={{ rowGap: 18 }}>
            <StatTile label="Time in bed" value={fmtHM(d.stats.timeInBedMin)} unit="h" />
            <StatTile label="Time asleep" value={fmtHM(d.stats.asleepMin)} unit="h" />
            <StatTile label="Efficiency" value={fmt(d.stats.efficiencyPct)} unit="%" />
            <StatTile label="Latency" value={fmt(d.stats.latencyMin)} unit="min" />
            <StatTile label="Disturbances" value={fmt(d.stats.disturbances)} />
            <StatTile label="Respiratory" value={fmt(d.stats.respRate, 1)} unit="rpm" />
            <StatTile label="Bedtime" value={fmtTime(d.stats.bedtimeMs)} />
            <StatTile label="Wake" value={fmtTime(d.stats.wakeMs)} />
          </div>
        </Card>
      </div>

      <div className="grid g2">
        <Card title="Sleep need">
          <StackedBar
            rows={[{ name: "Need", parts: needParts }, { name: "Got", parts: [{ label: "Asleep", value: d.need.actualMin, color: "var(--stage-deep)" }] }]}
            formatValue={(v) => fmtHM(v)} height={100}
          />
          <p className="muted num" style={{ marginTop: 6 }}>
            Baseline {fmtHM(d.need.baselineMin)} + strain {fmtHM(d.need.strainMin)} + debt {fmtHM(d.need.debtMin)} − naps {fmtHM(d.need.napsMin)} = <b>{fmtHM(d.need.totalMin)}</b> needed · you slept {fmtHM(d.need.actualMin)}
          </p>
        </Card>
        <Card title="Sleep planner">
          <ResultView result={d.planner}>
            {(p) => (
              <div className="col">
                <p className="muted">Bedtimes tonight to wake at {fmtTime(p.wakeTimeMs)} (typical efficiency {p.typicalEfficiencyPct}%).</p>
                <div className="grid g3">
                  {p.options.map((o) => (
                    <StatTile key={o.goal} label={o.goal === "peak" ? "Peak" : o.goal === "perform" ? "Perform" : "Get by"} value={fmtTime(o.bedtimeMs)} sub={`${o.goalPct}% · ${fmtHM(o.sleepMin)} h`} />
                  ))}
                </div>
              </div>
            )}
          </ResultView>
        </Card>
      </div>

      <Card title="Consistency · last 14 nights" action={{ label: "See trends", onClick: () => nav.go("trends") }}>
        <TimeRangeBars rows={d.consistency.map((c) => ({ label: formatDayShort(c.day), startMs: c.bedMs, endMs: c.wakeMs }))} />
      </Card>
    </>
  );
}
