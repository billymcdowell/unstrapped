import * as api from "../api/client";
import type { RecoveryData } from "../api/types";
import { Card, Dial, EmptyState, ResultView, DailyBars } from "../components";
import { useNav } from "../lib/app-context";
import { useApi } from "../lib/hooks";
import { formatDayShort } from "../lib/date";
import { bandVar, fmt } from "../lib/format";

export function Recovery() {
  const nav = useNav();
  const { data, error } = useApi(() => api.getRecovery(nav.day), [nav.day]);
  if (error && !data) return <div className="page"><EmptyState title="Couldn't load" detail={error} /></div>;
  return (
    <div className="page">
      <h1 className="page-title">Recovery</h1>
      <ResultView result={data}>{(d) => <RecoveryBody d={d} />}</ResultView>
    </div>
  );
}

function RecoveryBody({ d }: { d: RecoveryData }) {
  const nav = useNav();
  const max = Math.max(5, ...d.contributors.map((c) => Math.abs(c.impactPts)));
  const dirColor = (x: string) => (x === "good" ? "var(--ok)" : x === "bad" ? "var(--alert)" : "var(--text-3)");
  return (
    <>
      <Card>
        <div className="row" style={{ gap: 32 }}>
          <Dial label="Recovery" color={bandVar(d.band)} value={d.score} size={180} />
          <div className="col" style={{ gap: 6 }}>
            <span className="pill" style={{ background: bandVar(d.band), color: "#fff", alignSelf: "flex-start" }}>{d.band.toUpperCase()}</span>
            <p style={{ fontSize: 18, fontWeight: 600 }}>{d.reading}</p>
            <p className="muted">Red 0–33 · Yellow 34–66 · Green 67–100</p>
          </div>
        </div>
      </Card>

      <div className="grid g4">
        {d.contributors.map((c) => (
          <Card key={c.key} className="pad-sm">
            <div className="stat">
              <span className="stat-label">{c.label}</span>
              <span className="stat-value num">{fmt(c.value, c.unit === "rpm" ? 1 : 0)}<span className="stat-unit">{c.unit}</span></span>
              <span className="stat-sub num">baseline {fmt(c.baseline, c.unit === "rpm" ? 1 : 0)}</span>
              <span className="num" style={{ color: dirColor(c.direction), fontWeight: 650 }} aria-label={`${c.direction} change`}>
                {c.deltaPct >= 0 ? "▲" : "▼"} {Math.abs(c.deltaPct).toFixed(1)}%
              </span>
            </div>
          </Card>
        ))}
      </div>

      <div className="grid g2">
        <Card title="What moved your score">
          <div className="col" style={{ gap: 14 }}>
            {d.contributors.map((c) => (
              <div key={c.key} className="bar-row">
                <span className="bar-label">{c.label}</span>
                <span className="bar-val num" style={{ color: c.impactPts >= 0 ? "var(--ok)" : "var(--alert)" }}>{c.impactPts >= 0 ? "+" : "−"}{Math.abs(c.impactPts).toFixed(1)} pts</span>
                <div className="bar-track" style={{ background: "var(--band-track)" }}>
                  <div style={{ position: "absolute", left: "50%", top: 0, bottom: 0, width: 1, background: "var(--hairline)" }} />
                  <div className="bar-fill" style={{
                    position: "absolute", top: 0, bottom: 0, borderRadius: 0,
                    width: `${(Math.abs(c.impactPts) / max) * 50}%`,
                    left: c.impactPts >= 0 ? "50%" : `${50 - (Math.abs(c.impactPts) / max) * 50}%`,
                    background: c.impactPts >= 0 ? "var(--ok)" : "var(--alert)",
                  }} />
                </div>
              </div>
            ))}
          </div>
        </Card>
        <Card title="Last 7 days" action={{ label: "See trends", onClick: () => nav.go("trends") }}>
          <DailyBars
            height={200} unit="%" yMax={100} yMin={0}
            points={d.last7.map((x) => ({ label: formatDayShort(x.day), value: x.score, color: bandVar(x.band) }))}
          />
        </Card>
      </div>
    </>
  );
}
