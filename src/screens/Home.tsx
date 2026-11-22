import type { Result } from "../api/types";
import * as api from "../api/client";
import { ActivityList, AddActivity, Card, Dial, EmptyState, ResultView, ScoreTile, StatTile, StatusIcon, TimeSeriesChart } from "../components";
import { useNav } from "../lib/app-context";
import { useApi } from "../lib/hooks";
import { fmtTime, fmtHM, formatDayLong, today } from "../lib/date";
import { bandTone, fmt, strainBandLabel, stressBandLabel } from "../lib/format";

const STRESS_BANDS = [
  { from: 0, to: 1, color: "rgba(74,148,89,0.07)" },
  { from: 1, to: 2, color: "rgba(214,150,42,0.08)" },
  { from: 2, to: 3, color: "rgba(212,85,60,0.08)" },
];
const BAND_WORD = { green: "Primed to perform", yellow: "Maintaining", red: "Take it easy" } as const;

/** Caption for a tile whose result isn't ok. */
const stateCaption = (r: Result<unknown>) => (r.state === "calibrating" ? `Calibrating — ${r.nightsRemaining} more night${r.nightsRemaining === 1 ? "" : "s"}` : r.state === "noData" ? "No data" : "");

const greeting = () => { const h = new Date().getHours(); return h < 12 ? "Good morning" : h < 18 ? "Good afternoon" : "Good evening"; };

export function Home() {
  const nav = useNav();
  const { data: h, error } = useApi(() => api.getHome(nav.day), [nav.day]);
  const isToday = nav.day === today();
  const head = (
    <div>
      <h1 className="page-title">{isToday ? greeting() : formatDayLong(nav.day)}</h1>
      <p className="page-sub">{isToday ? "Here's how your body is doing today." : "A look back at this day."}</p>
    </div>
  );
  if (!h) return <div className="page">{head}{error ? <EmptyState title="Couldn't load" detail={error} /> : <div className="empty faint">Loading…</div>}</div>;

  const s = h.sleep, r = h.recovery, st = h.strain;
  return (
    <div className="page">
      {head}
      <div className="grid g3">
        <ScoreTile
          tone="dusk" label="Sleep" onClick={() => nav.go("sleep")}
          value={s.state === "ok" ? Math.round(s.data.performance) : "—"} unit={s.state === "ok" ? "%" : undefined}
          caption={s.state === "ok" ? "Sleep performance" : stateCaption(s)}
        >
          <Dial bare color="var(--text)" value={s.state === "ok" ? s.data.performance : null} size={170} label="Sleep" />
        </ScoreTile>
        <ScoreTile
          tone={r.state === "ok" ? bandTone(r.data.band) : "dusk"} label="Recovery" onClick={() => nav.go("recovery")}
          value={r.state === "ok" ? Math.round(r.data.score) : "—"} unit={r.state === "ok" ? "%" : undefined}
          caption={r.state === "ok" ? BAND_WORD[r.data.band] : stateCaption(r)}
        >
          <Dial bare color="var(--text)" value={r.state === "ok" ? r.data.score : null} size={170} label="Recovery" />
        </ScoreTile>
        <ScoreTile
          tone="ochre" label="Strain" onClick={() => nav.go("strain")}
          value={st.state === "ok" ? st.data.value.toFixed(1) : "—"}
          caption={st.state === "ok" ? `${strainBandLabel[st.data.band]}${st.data.targetLo !== null ? ` · target ${st.data.targetLo}–${st.data.targetHi}` : ""}` : stateCaption(st)}
        >
          <Dial
            bare color="var(--text)" max={21} size={170} label="Strain"
            value={st.state === "ok" ? st.data.value : null}
            target={st.state === "ok" && st.data.targetLo !== null && st.data.targetHi !== null ? { lo: st.data.targetLo, hi: st.data.targetHi } : null}
          />
        </ScoreTile>
      </div>

      <div className="grid g-main">
        <Card title="Stress since waking" action={{ label: "Open", onClick: () => nav.go("stress") }}>
          <ResultView result={h.stress}>
            {(d) => (
              <>
                <div className="row" style={{ alignItems: "baseline", marginBottom: 8 }}>
                  <span className="stat-value num" style={{ fontSize: 40 }}>{fmt(d.current, 1)}</span>
                  {d.band ? <span className="pill">{stressBandLabel[d.band]}</span> : <span className="faint">No current reading</span>}
                </div>
                {d.series.ts.length ? <TimeSeriesChart series={d.series} height={130} color="var(--stress)" yRange={[0, 3]} bands={STRESS_BANDS} ariaLabel="Stress since waking" /> : <EmptyState title="No stress readings yet" />}
              </>
            )}
          </ResultView>
        </Card>
        <Card title="Tonight's sleep" action={{ label: "Planner", onClick: () => nav.go("sleep") }}>
          <ResultView result={h.tonightsSleep}>
            {(t) => (
              <div className="col" style={{ gap: 20 }}>
                <StatTile label="Sleep need" value={fmtHM(t.needMin)} unit="h" />
                <StatTile label="Recommended bedtime" value={fmtTime(t.recommendedBedtimeMs)} sub={`to wake at ${fmtTime(t.wakeTimeMs)}`} />
              </div>
            )}
          </ResultView>
        </Card>
      </div>

      <Card title="Vitals" action={{ label: "Health monitor", onClick: () => nav.go("health") }}>
        <ResultView result={h.healthMonitor}>
          {(m) => (
            <div className="row wrap" style={{ marginBottom: 20 }}>
              <StatusIcon level={m.flagged.length === 0 ? "ok" : m.flagged.some((f) => f.status === "alert") ? "alert" : "warn"} />
              <span>{m.inRange} of {m.total} metrics in range</span>
              {m.flagged.map((f) => (
                <span key={f.metric} className="pill"><StatusIcon level={f.status} size={12} />{f.label}</span>
              ))}
            </div>
          )}
        </ResultView>
        <ResultView result={h.keyStats}>
          {(k) => (
            <div className="grid g4">
              <StatTile label="HRV" value={fmt(k.hrvMs)} unit="ms" />
              <StatTile label="Resting heart rate" value={fmt(k.restingHr)} unit="bpm" />
              <StatTile label="Respiratory rate" value={fmt(k.respRate, 1)} unit="rpm" />
              <StatTile label="Calories" value={fmt(k.calories)} unit="kcal" />
            </div>
          )}
        </ResultView>
      </Card>

      <Card title="Activities">
        <ActivityList activities={h.activities} onOpen={(id) => nav.openActivity(id)} />
        <div style={{ marginTop: 12 }}><AddActivity day={nav.day} /></div>
      </Card>

      <div className="row">
        <button className="btn" onClick={() => nav.go("journal")}>Open journal</button>
        {h.gaps.map((g, i) => <span key={i} className="faint num">{g.reason} {fmtTime(g.startMs)}–{fmtTime(g.endMs)}</span>)}
      </div>
    </div>
  );
}
