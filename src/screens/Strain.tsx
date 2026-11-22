import { useEffect, useState } from "react";
import * as api from "../api/client";
import type { StrainData } from "../api/types";
import { ActivityList, AddActivity, Card, Dial, EmptyState, ResultView, StatTile, TimeSeriesChart, ZoneBars } from "../components";
import { useNav } from "../lib/app-context";
import { useApi } from "../lib/hooks";
import { fmtDuration, fmtTime } from "../lib/date";
import { fmt, hrZones, strainBandLabel } from "../lib/format";

export function Strain() {
  const nav = useNav();
  if (nav.activityId) return <ActivityDetailView id={nav.activityId} />;
  return <StrainDay />;
}

function StrainDay() {
  const nav = useNav();
  const { data, error } = useApi(() => api.getStrain(nav.day), [nav.day]);
  if (error && !data) return <div className="page"><EmptyState title="Couldn't load" detail={error} /></div>;
  return (
    <div className="page">
      <h1 className="page-title">Strain</h1>
      <ResultView result={data}>{(d) => <StrainBody d={d} />}</ResultView>
    </div>
  );
}

function StrainBody({ d }: { d: StrainData }) {
  const nav = useNav();
  const hasTarget = d.targetLo !== null && d.targetHi !== null;
  const progress = hasTarget && d.targetLo! > 0 ? Math.min(100, Math.round((d.value / d.targetLo!) * 100)) : null;
  return (
    <>
      <Card>
        <div className="row" style={{ gap: 32 }}>
          <Dial
            label="Day strain" color="var(--strain)" max={21} value={d.value} size={180} sub={strainBandLabel[d.band]}
            target={hasTarget ? { lo: d.targetLo!, hi: d.targetHi! } : null}
          />
          <div className="col" style={{ gap: 6 }}>
            <span className="pill" style={{ alignSelf: "flex-start" }}>{strainBandLabel[d.band]}</span>
            {hasTarget ? (
              <>
                <p style={{ fontSize: 18, fontWeight: 600 }}>Target {d.targetLo}–{d.targetHi}</p>
                <p className="muted">{progress !== null && progress >= 100 ? "You've reached your target range." : `${progress ?? 0}% of the way to the bottom of your target range.`}</p>
              </>
            ) : <p className="muted">No target yet — it's set from your recovery.</p>}
            <p className="faint">Light 0–9 · Moderate 10–13 · High 14–17 · All out 18–21</p>
          </div>
        </div>
      </Card>

      <Card title="Heart rate">
        <TimeSeriesChart
          series={d.hr} height={230} zones={hrZones(d.zoneBounds)} spans={d.spans}
          xRange={[d.hr.ts[0], d.hr.ts[0] + 86_400_000]} yRange={[40, d.maxHr]} ariaLabel="Heart rate over the day"
        />
        <div className="legend" style={{ marginTop: 6 }}>
          <span><i style={{ background: "var(--span-sleep)" }} />Sleep</span>
          <span><i style={{ background: "var(--span-activity)" }} />Activity</span>
          <span><i style={{ background: "var(--span-gap)" }} />Strap off</span>
        </div>
        {d.gaps.map((g, i) => <div key={i} className="faint num">{g.reason} {fmtTime(g.startMs)}–{fmtTime(g.endMs)}</div>)}
      </Card>

      <div className="grid g2">
        <Card title="Time in zones"><ZoneBars zoneMin={d.zoneMin} bounds={d.zoneBounds} /></Card>
        <Card title="Day stats">
          <div className="grid g2" style={{ rowGap: 18 }}>
            <StatTile label="Calories" value={fmt(d.stats.calories)} unit="kcal" />
            <StatTile label="Average HR" value={fmt(d.stats.avgHr)} unit="bpm" />
            <StatTile label="Steps" value={d.stats.steps === null ? "—" : fmt(d.stats.steps)} sub={d.stats.steps === null ? "Not reported by this strap" : undefined} />
            <StatTile label="Strength time" value={fmtDuration(d.stats.strengthMin)} sub="logged manually" />
          </div>
        </Card>
      </div>

      <Card title="Activities" action={{ label: "See trends", onClick: () => nav.go("trends") }}>
        <ActivityList activities={d.activities} onOpen={(id) => nav.openActivity(id)} />
        <div style={{ marginTop: 10 }}><AddActivity day={nav.day} /></div>
      </Card>
    </>
  );
}

function ActivityDetailView({ id }: { id: string }) {
  const nav = useNav();
  const { data, error } = useApi(() => api.getActivity(id), [id]);
  const [name, setName] = useState("");
  const [saving, setSaving] = useState(false);
  useEffect(() => { if (data) setName(data.summary.kind); }, [data]);

  if (error && !data) return <div className="page"><button className="btn sm" onClick={() => nav.openActivity(null)}>◀ Back</button><EmptyState title="Couldn't load activity" detail={error} /></div>;
  if (!data) return <div className="page"><div className="empty faint">Loading…</div></div>;
  const a = data.summary;
  const rename = async () => {
    if (!name.trim() || name === a.kind) return;
    setSaving(true);
    try { await api.renameActivity(a.id, name.trim()); nav.refresh(); } finally { setSaving(false); }
  };
  return (
    <div className="page">
      <div className="row">
        <button className="btn sm" onClick={() => nav.openActivity(null)}>◀ Back to strain</button>
      </div>
      <div className="row wrap">
        <input aria-label="Activity type" className="page-title" value={name} onChange={(e) => setName(e.target.value)} onBlur={rename}
          onKeyDown={(e) => e.key === "Enter" && (e.currentTarget.blur())}
          style={{ background: "none", border: 0, borderBottom: "1px dashed var(--hairline)", width: 260, userSelect: "text" }} />
        {saving && <span className="faint">Saving…</span>}
        <span className="muted num">{fmtTime(a.startMs)}–{fmtTime(a.endMs)} · {fmtDuration((a.endMs - a.startMs) / 60_000)}</span>
      </div>
      <div className="grid g-main">
        <Card title="Heart rate">
          <TimeSeriesChart series={data.hr} height={230} zones={hrZones(data.zoneBounds)} yRange={[40, data.maxHr]} ariaLabel="Heart rate during activity" />
        </Card>
        <Card><Dial label="Activity strain" color="var(--strain)" max={21} value={a.strain} sub={strainBandLabel[a.strain >= 18 ? "allOut" : a.strain >= 14 ? "high" : a.strain >= 10 ? "moderate" : "light"]} /></Card>
      </div>
      <div className="grid g2">
        <Card title="Time in zones"><ZoneBars zoneMin={a.zoneMin} bounds={data.zoneBounds} /></Card>
        <Card title="Stats">
          <div className="grid g2" style={{ rowGap: 18 }}>
            <StatTile label="Duration" value={fmtDuration((a.endMs - a.startMs) / 60_000)} />
            <StatTile label="Calories" value={fmt(a.calories)} unit="kcal" />
            <StatTile label="Average HR" value={fmt(a.avgHr)} unit="bpm" />
            <StatTile label="Max HR" value={fmt(a.maxHr)} unit="bpm" />
          </div>
        </Card>
      </div>
    </div>
  );
}
