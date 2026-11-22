import type { Result } from "../api/types";
import * as api from "../api/client";
import { ActivityList, AddActivity, Card, Dial, EmptyState, ResultView, StatTile, StatusIcon, TimeSeriesChart } from "../components";
import { useNav } from "../lib/app-context";
import { useApi } from "../lib/hooks";
import { fmtTime, fmtHM } from "../lib/date";
import { bandVar, fmt, strainBandLabel, stressBandLabel } from "../lib/format";

const STRESS_BANDS = [
  { from: 0, to: 1, color: "rgba(37,184,102,0.08)" },
  { from: 1, to: 2, color: "rgba(233,168,0,0.09)" },
  { from: 2, to: 3, color: "rgba(229,72,77,0.1)" },
];

/** Caption for a dial whose result isn't ok. */
const stateCaption = (r: Result<unknown>) => (r.state === "calibrating" ? `Calibrating — ${r.nightsRemaining} more night${r.nightsRemaining === 1 ? "" : "s"}` : r.state === "noData" ? "No data" : "");

export function Home() {
  const nav = useNav();
  const { data: h, error } = useApi(() => api.getHome(nav.day), [nav.day]);
  if (!h) return <div className="page">{error ? <EmptyState title="Couldn't load" detail={error} /> : <div className="empty faint">Loading…</div>}</div>;

  const s = h.sleep, r = h.recovery, st = h.strain;
  return (
    <div className="page">
      <div className="grid g3">
        <Card onClick={() => nav.go("sleep")}>
          <Dial label="Sleep" color="var(--sleep)" value={s.state === "ok" ? s.data.performance : null} caption={stateCaption(s)} />
        </Card>
        <Card onClick={() => nav.go("recovery")}>
          <Dial label="Recovery" color={r.state === "ok" ? bandVar(r.data.band) : "var(--text-3)"} value={r.state === "ok" ? r.data.score : null} caption={stateCaption(r)} />
        </Card>
        <Card onClick={() => nav.go("strain")}>
          <Dial
            label="Strain" color="var(--strain)" max={21}
            value={st.state === "ok" ? st.data.value : null}
            sub={st.state === "ok" ? strainBandLabel[st.data.band] : undefined}
            target={st.state === "ok" && st.data.targetLo !== null && st.data.targetHi !== null ? { lo: st.data.targetLo, hi: st.data.targetHi } : null}
            caption={st.state === "ok" ? (st.data.targetLo !== null ? `target ${st.data.targetLo}–${st.data.targetHi}` : "") : stateCaption(st)}
          />
        </Card>
      </div>

      <Card title="Health Monitor" action={{ label: "Details", onClick: () => nav.go("health") }}>
        <ResultView result={h.healthMonitor}>
          {(m) => (
            <div className="row wrap">
              <StatusIcon level={m.flagged.length === 0 ? "ok" : m.flagged.some((f) => f.status === "alert") ? "alert" : "warn"} />
              {m.flagged.length === 0 ? (
                <span>{m.inRange} of {m.total} metrics in range</span>
              ) : (
                <>
                  <span>{m.inRange} of {m.total} in range ·</span>
                  {m.flagged.map((f) => (
                    <span key={f.metric} className="row" style={{ gap: 5 }}><StatusIcon level={f.status} size={14} />{f.label}</span>
                  ))}
                </>
              )}
            </div>
          )}
        </ResultView>
      </Card>

      <div className="grid g-main">
        <Card title="Stress" action={{ label: "Open", onClick: () => nav.go("stress") }}>
          <ResultView result={h.stress}>
            {(d) => (
              <>
                <div className="row" style={{ marginBottom: 6 }}>
                  <span className="stat-value num">{fmt(d.current, 1)}</span>
                  <span className="muted">{d.band ? `${stressBandLabel[d.band]} · since waking` : "No current reading"}</span>
                </div>
                {d.series.ts.length ? <TimeSeriesChart series={d.series} height={120} color="var(--stress)" yRange={[0, 3]} bands={STRESS_BANDS} ariaLabel="Stress since waking" /> : <EmptyState title="No stress readings yet" />}
              </>
            )}
          </ResultView>
        </Card>
        <Card title="Tonight's sleep" action={{ label: "Planner", onClick: () => nav.go("sleep") }}>
          <ResultView result={h.tonightsSleep}>
            {(t) => (
              <div className="col">
                <StatTile label="Sleep need" value={fmtHM(t.needMin)} unit="h" />
                <StatTile label="Recommended bedtime" value={fmtTime(t.recommendedBedtimeMs)} sub={`to wake at ${fmtTime(t.wakeTimeMs)}`} />
              </div>
            )}
          </ResultView>
        </Card>
      </div>

      <Card title="Activities">
        <ActivityList activities={h.activities} onOpen={(id) => nav.openActivity(id)} />
        <div style={{ marginTop: 10 }}><AddActivity day={nav.day} /></div>
      </Card>

      <Card>
        <ResultView result={h.keyStats}>
          {(k) => (
            <div className="grid g4">
              <StatTile label="HRV" value={fmt(k.hrvMs)} unit="ms" />
              <StatTile label="Resting HR" value={fmt(k.restingHr)} unit="bpm" />
              <StatTile label="Respiratory" value={fmt(k.respRate, 1)} unit="rpm" />
              <StatTile label="Calories" value={fmt(k.calories)} unit="kcal" />
            </div>
          )}
        </ResultView>
      </Card>

      <div className="row">
        <button className="btn" onClick={() => nav.go("journal")}>Open journal</button>
        {h.gaps.map((g, i) => <span key={i} className="faint num">{g.reason} {fmtTime(g.startMs)}–{fmtTime(g.endMs)}</span>)}
      </div>
    </div>
  );
}
