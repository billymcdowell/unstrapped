import { useMemo, useState } from "react";
import * as api from "../api/client";
import type { Assessment, AssessmentListItem, AssessmentKind } from "../api/types";
import { CalendarHeatmap, Card, ContributorBar, DivergingBars, EmptyState, ScatterPlot, StackedBar, StatTile, TimeRangeBars, TrainingResponseChart } from "../components";
import { useApi, useColorScheme } from "../lib/hooks";
import { fmtDuration, formatDayShort } from "../lib/date";
import { ZONE_COLORS, cssVar, fmt } from "../lib/format";

const KIND_LABEL: Record<AssessmentKind, string> = { weekly: "Weekly", monthly: "Monthly", yearly: "Yearly" };

export function Assessments() {
  const { data, error } = useApi(() => api.listAssessments(), []);
  const [picked, setPicked] = useState<string | null>(null);
  const id = picked ?? data?.[0]?.id ?? null;
  return (
    <div className="page">
      <div>
        <h1 className="page-title">Performance Assessments</h1>
        <p className="page-sub">Reports are created each Monday, on the 1st of the month and once a year.</p>
      </div>
      {error && !data ? <EmptyState title="Couldn't load" detail={error} /> : !data ? <div className="empty faint">Loading…</div> : data.length === 0 ? (
        <Card><EmptyState title="No reports yet" detail="The first weekly report appears after your first full week of data." /></Card>
      ) : (
        <div className="split">
          <Card className="picker pad-sm" title="Past reports">
            {data.map((a) => <ListItem key={a.id} a={a} active={a.id === id} onPick={() => setPicked(a.id)} />)}
          </Card>
          <div className="col" style={{ gap: "var(--s-4)", minWidth: 0 }}>{id && <Detail id={id} />}</div>
        </div>
      )}
    </div>
  );
}

function ListItem({ a, active, onPick }: { a: AssessmentListItem; active: boolean; onPick: () => void }) {
  return (
    <button className="pick report" aria-pressed={active} onClick={onPick}>
      <span className="col" style={{ gap: 0, alignItems: "flex-start" }}>
        <b>{a.title}</b>
        <span className="faint num">{formatDayShort(a.periodStart)} – {formatDayShort(a.periodEnd)}</span>
      </span>
      <span className="pill">{KIND_LABEL[a.kind]}</span>
    </button>
  );
}

function Detail({ id }: { id: string }) {
  const { data, error } = useApi(() => api.getAssessment(id), [id]);
  if (error && !data) return <EmptyState title="Couldn't load report" detail={error} />;
  if (!data) return <div className="empty faint">Loading…</div>;
  return <Report a={data} />;
}

const nodata = (n: number | null, d = 0, unit = "") => (n === null ? "—" : `${fmt(n, d)}${unit}`);

function Report({ a }: { a: Assessment }) {
  const scheme = useColorScheme();
  const days = a.scatter.length;
  const noWeekly = a.avgRecovery === null && a.avgStrain === null && a.avgSleepPerformance === null && !a.heat;
  const heatColors = useMemo(() => ({
    recovery: [cssVar("--rec-red"), cssVar("--rec-yellow"), cssVar("--rec-green")],
    strain: [`${cssVar("--strain")}33`, cssVar("--strain")],
    sleep: [`${cssVar("--sleep")}33`, cssVar("--sleep")],
  }), [scheme]);
  const mono = a.kind !== "yearly";

  return (
    <>
      <Card>
        <div className="row wrap" style={{ gap: 12 }}>
          <div>
            <h2 style={{ fontSize: 18, fontWeight: 650 }}>{a.title}</h2>
            <span className="muted num">{formatDayShort(a.periodStart)} – {formatDayShort(a.periodEnd)}</span>
          </div>
          <span className="spacer" />
          <span className="pill">{KIND_LABEL[a.kind]}</span>
        </div>
        {noWeekly ? <EmptyState title="Not enough data for this period" detail="No scored days were recorded." /> : (
          <div className="grid g3" style={{ marginTop: 16 }}>
            <StatTile label="Average recovery" value={nodata(a.avgRecovery, 0)} unit={a.avgRecovery === null ? undefined : "%"} />
            <StatTile label="Average strain" value={nodata(a.avgStrain, 1)} />
            <StatTile label="Average sleep performance" value={nodata(a.avgSleepPerformance, 0)} unit={a.avgSleepPerformance === null ? undefined : "%"} />
          </div>
        )}
      </Card>

      {mono && (
        <>
          <div className="grid g2">
            <Card title="Hours slept vs needed">
              {a.hoursNeeded > 0 ? (
                <>
                  <ContributorBar label="Slept" value={a.hoursSlept} max={Math.max(a.hoursNeeded, a.hoursSlept)} valueText={`${fmt(a.hoursSlept, 1)} h of ${fmt(a.hoursNeeded, 1)} h`} color="var(--sleep)" />
                  <p className="muted" style={{ marginTop: 10 }}>
                    {a.hoursSlept >= a.hoursNeeded ? `${fmt(a.hoursSlept - a.hoursNeeded, 1)} h more than you needed.` : `${fmt(a.hoursNeeded - a.hoursSlept, 1)} h short of what you needed.`}
                  </p>
                </>
              ) : <EmptyState title="No sleep need recorded" />}
            </Card>
            <Card title="Bed and wake times">
              {a.consistency.length ? <TimeRangeBars rows={a.consistency.map((c) => ({ label: formatDayShort(c.day), startMs: c.bedMs, endMs: c.wakeMs }))} /> : <EmptyState title="No nights recorded" />}
            </Card>
          </div>
          <Card title="Day strain vs next-day recovery">
            {days >= 3 ? (
              <>
                <ScatterPlot points={a.scatter.map((p) => ({ x: p.strain, y: p.nextRecovery, label: formatDayShort(p.day) }))} xName="Day strain" yName="Next-day recovery %" xMax={21} yMax={100} />
                <p className="faint">Each dot is one day. A downward slope means harder days cost you recovery.</p>
              </>
            ) : <EmptyState title="Not enough days" detail="At least three days with a following recovery score are needed." />}
          </Card>
        </>
      )}

      {a.kind !== "weekly" && a.zonesByActivity && (
        <Card title="Strain and zone time by activity">
          {a.zonesByActivity.some((z) => z.zoneMin.some((m) => m > 0)) ? (
            <>
              <StackedBar
                rows={a.zonesByActivity.filter((z) => z.zoneMin.some((m) => m > 0)).map((z) => ({ name: z.kind, parts: z.zoneMin.map((m, i) => ({ label: `Zone ${i + 1}`, value: m, color: ZONE_COLORS[i] })) }))}
                formatValue={(v) => (v >= 60 ? fmtDuration(v) : `${Math.round(v)}m`)}
              />
              <div className="list" style={{ marginTop: 8 }}>
                {a.zonesByActivity.filter((z) => z.zoneMin.some((m) => m > 0)).map((z) => (
                  <div key={z.kind} className="list-row">
                    <span style={{ fontWeight: 600, minWidth: 90 }}>{z.kind}</span>
                    <span className="spacer" />
                    <span className="muted num">{z.sessions !== undefined ? `${z.sessions} session${z.sessions === 1 ? "" : "s"}` : ""}</span>
                    <span className="num">{z.avgStrain === undefined || z.avgStrain === null ? "—" : <><b>{fmt(z.avgStrain, 1)}</b> <span className="faint">avg strain</span></>}</span>
                    <span className="muted num">{fmtDuration(z.zoneMin.reduce((s, m) => s + m, 0))} total</span>
                  </div>
                ))}
              </div>
            </>
          ) : <EmptyState title="No activities in this period" />}
        </Card>
      )}

      {a.kind !== "weekly" && a.topJournalImpacts && (
        <Card title="Top journal impacts on recovery">
          {(() => {
            const rows = a.topJournalImpacts.filter((i) => i.state === "ok" && i.recoveryDelta !== null);
            return rows.length ? (
              <DivergingBars rows={[...rows].sort((x, y) => (y.recoveryDelta as number) - (x.recoveryDelta as number)).map((i) => ({ key: i.behavior, label: i.label, value: i.recoveryDelta as number, sub: `${i.yesDays} yes / ${i.noDays} no` }))} />
            ) : <EmptyState title="Not enough journal data yet" detail="Behaviours need 5 yes and 5 no answers." />;
          })()}
        </Card>
      )}

      {a.kind !== "weekly" && a.trainingResponse && (
        <Card title="Training and response, 6 months">
          {a.trainingResponse.months.length ? (
            <>
              <TrainingResponseChart {...a.trainingResponse} />
              <div className="legend"><span><i style={{ background: "var(--strain)" }} />Strain (left axis)</span><span><i style={{ background: "var(--rec-green)" }} />Recovery (right axis)</span><span><i style={{ background: "var(--stress)" }} />Resting HR</span></div>
              <p className="faint">Strain bars with recovery above, resting heart rate below. Rising strain with steady recovery and falling resting HR suggests fitness is improving.</p>
            </>
          ) : <EmptyState title="Not enough history" />}
        </Card>
      )}

      {a.heat && (
        <>
          <Card title="Recovery by day"><CalendarHeatmap data={a.heat.recovery} from={a.periodStart} to={a.periodEnd} min={0} max={100} colors={heatColors.recovery} unit="%" /></Card>
          <Card title="Strain by day"><CalendarHeatmap data={a.heat.strain} from={a.periodStart} to={a.periodEnd} min={0} max={21} colors={heatColors.strain} /></Card>
          <Card title="Sleep performance by day"><CalendarHeatmap data={a.heat.sleep} from={a.periodStart} to={a.periodEnd} min={0} max={100} colors={heatColors.sleep} unit="%" /></Card>
        </>
      )}
    </>
  );
}
