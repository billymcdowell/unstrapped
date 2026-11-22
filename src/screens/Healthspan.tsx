import * as api from "../api/client";
import type { Healthspan as HealthspanData } from "../api/types";
import { Card, EmptyState, ResultView } from "../components";
import { useApi } from "../lib/hooks";
import { clamp, fmt } from "../lib/format";

const MINUS = "−";
const years = (n: number) => `${n > 0 ? "+" : n < 0 ? MINUS : ""}${Math.abs(n).toFixed(1)} y`;

export function Healthspan() {
  const { data, error } = useApi(() => api.getHealthspan(), []);
  return (
    <div className="page">
      <div>
        <h1 className="page-title">Healthspan</h1>
        <p className="page-sub">How your habits compare with your age.</p>
      </div>
      {error && !data ? <EmptyState title="Couldn't load" detail={error} /> : <ResultView result={data}>{(d) => <Body d={d} />}</ResultView>}
      <div className="note" role="note">
        <b>Approximate.</b> This is a motivator, not a measurement. It is a rough estimate built from wrist-sensor data and population averages, and it is not a medical assessment.
      </div>
    </div>
  );
}

function Body({ d }: { d: HealthspanData }) {
  const diff = Math.round((d.strapAge - d.chronologicalAge) * 10) / 10;
  const younger = diff < 0;
  const color = diff === 0 ? "var(--text)" : younger ? "var(--ok)" : "var(--warn)";
  return (
    <>
      <div className="grid g2">
        <Card title="Physiological age">
          <div className="row" style={{ gap: 28, alignItems: "flex-end" }}>
            <div className="stat"><span className="stat-label">Strap age</span><span className="num" style={{ fontSize: 64, fontWeight: 700, lineHeight: 1, color }}>{fmt(d.strapAge, 1)}</span></div>
            <div className="stat"><span className="stat-label">Your age</span><span className="num" style={{ fontSize: 32, fontWeight: 650, color: "var(--text-2)" }}>{fmt(d.chronologicalAge, 0)}</span></div>
          </div>
          <p style={{ marginTop: 10, fontSize: 15 }}>
            {diff === 0 ? "About the same as your age." : <><b style={{ color }}>{Math.abs(diff).toFixed(1)} years {younger ? "younger" : "older"}</b> than your age.</>}
          </p>
        </Card>
        <Card title="Pace of aging">
          <Gauge pace={d.paceOfAging} />
        </Card>
      </div>
      <Card title="What drives it">
        <div className="list">
          <div className="list-row contrib-head faint">
            <span style={{ flex: 1 }}>Contributor</span><span className="contrib-col">6-month average</span><span className="contrib-col">Target</span><span className="contrib-col right">Years</span>
          </div>
          {d.contributors.map((c) => (
            <div key={c.key} className="list-row">
              <span style={{ flex: 1, fontWeight: 550 }}>{c.label}</span>
              <span className="contrib-col num">{c.average}</span>
              <span className="contrib-col num muted">{c.target}</span>
              <span className="contrib-col right num" style={{ fontWeight: 650, color: c.yearsImpact > 0 ? "var(--ok)" : c.yearsImpact < 0 ? "var(--alert)" : "var(--text-3)" }}>
                {years(c.yearsImpact)} <span className="faint" style={{ fontWeight: 500 }}>{c.yearsImpact > 0 ? "adds" : c.yearsImpact < 0 ? "removes" : ""}</span>
              </span>
            </div>
          ))}
        </div>
        <p className="faint" style={{ marginTop: 8 }}>Adds or removes years of physiological age. Green slows ageing, red speeds it up.</p>
      </Card>
    </>
  );
}

/** Half-circle gauge: 0.6 (slower) to 1.4 (faster), 1.0 = normal at the top. */
function Gauge({ pace }: { pace: number }) {
  const lo = 0.6, hi = 1.4, w = 260, cx = w / 2, cy = 130, r = 100;
  const ang = (v: number) => Math.PI * (1 - (clamp(v, lo, hi) - lo) / (hi - lo)); // pi (left) .. 0 (right)
  const pt = (v: number, rr = r) => [cx + rr * Math.cos(ang(v)), cy - rr * Math.sin(ang(v))];
  const arc = (a: number, b: number) => { const [x1, y1] = pt(a), [x2, y2] = pt(b); return `M${x1} ${y1} A${r} ${r} 0 0 1 ${x2} ${y2}`; };
  const [nx, ny] = pt(pace, r - 14);
  const slower = pace < 0.97, faster = pace > 1.03;
  return (
    <div className="col" style={{ alignItems: "center", gap: 4 }}>
      <svg width={w} height={150} viewBox={`0 0 ${w} 150`} role="img" aria-label={`Pace of aging ${pace.toFixed(2)}`}>
        <path d={arc(lo, 1)} fill="none" stroke="var(--ok)" strokeWidth={14} strokeLinecap="round" opacity={0.75} />
        <path d={arc(1, hi)} fill="none" stroke="var(--alert)" strokeWidth={14} strokeLinecap="round" opacity={0.75} />
        <line x1={cx} y1={cy} x2={nx} y2={ny} stroke="var(--text)" strokeWidth={3} strokeLinecap="round" />
        <circle cx={cx} cy={cy} r={6} fill="var(--text)" />
        <text x={cx - r} y={cy + 20} textAnchor="middle" className="dial-sub">{lo}</text>
        <text x={cx + r} y={cy + 20} textAnchor="middle" className="dial-sub">{hi}</text>
      </svg>
      <span className="num" style={{ fontSize: 30, fontWeight: 700 }}>{pace.toFixed(2)}<span className="stat-unit">years per year</span></span>
      <span className="muted">{slower ? "Ageing slower than the calendar." : faster ? "Ageing faster than the calendar." : "Ageing at a normal pace."}</span>
    </div>
  );
}
