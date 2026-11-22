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
  return (
    <>
      <div className="grid g2">
        <section className={`card tone ${younger || diff === 0 ? "tone-green" : "tone-coral"} score-tile`} style={{ minHeight: 300 }}>
          <div className="row" style={{ alignItems: "baseline" }}>
            <span className="score-label">Physiological age</span>
            <span className="spacer" />
            <span className="num" style={{ fontSize: 22, fontWeight: 300 }}>{diff === 0 ? "±0" : `${younger ? "−" : "+"}${Math.abs(diff).toFixed(1)}`}<span className="score-unit" style={{ fontSize: 13 }}>years</span></span>
          </div>
          <span className="score-value num">{fmt(d.strapAge, 1)}</span>
          <span className="score-caption">years · you are {fmt(d.chronologicalAge, 0)}</span>
          <div className="score-art"><AgeOrbit younger={younger} /></div>
          <p style={{ fontSize: 15 }}>
            {diff === 0 ? "About the same as your age." : <>{Math.abs(diff).toFixed(1)} years {younger ? "younger" : "older"} than your age.</>}
          </p>
        </section>
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
              <span style={{ flex: 1, fontWeight: 500 }}>{c.label}</span>
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
        <path d={arc(lo, 1)} fill="none" stroke="var(--ok)" strokeWidth={4} strokeLinecap="round" />
        <path d={arc(1, hi)} fill="none" stroke="var(--alert)" strokeWidth={4} strokeLinecap="round" />
        <line x1={cx} y1={cy} x2={nx} y2={ny} stroke="var(--text)" strokeWidth={1.5} strokeLinecap="round" />
        <circle cx={cx} cy={cy} r={4} fill="var(--text)" />
        <circle cx={nx} cy={ny} r={10} className="lime-halo" />
        <circle cx={nx} cy={ny} r={5} className="lime-dot" />
        <text x={cx - r} y={cy + 20} textAnchor="middle" className="dial-sub">{lo}</text>
        <text x={cx + r} y={cy + 20} textAnchor="middle" className="dial-sub">{hi}</text>
      </svg>
      <span className="num" style={{ fontSize: 40, fontWeight: 300, letterSpacing: "-0.03em" }}>{pace.toFixed(2)}<span className="stat-unit">years per year</span></span>
      <span className="muted">{slower ? "Ageing slower than the calendar." : faster ? "Ageing faster than the calendar." : "Ageing at a normal pace."}</span>
    </div>
  );
}

/** Decorative line art: stacked ellipses pinched at the centre, with a lime marker. */
function AgeOrbit({ younger }: { younger: boolean }) {
  const rings = Array.from({ length: 9 }, (_, i) => i);
  return (
    <svg width="220" height="120" viewBox="-110 -60 220 120" aria-hidden="true" fill="none" stroke="#fff" strokeWidth={0.8}>
      {rings.map((i) => <ellipse key={`a${i}`} cx={0} cy={-28 + i * 0.6} rx={30 + i * 9} ry={26 - i * 2.4} strokeOpacity={0.22 + i * 0.05} />)}
      {rings.map((i) => <ellipse key={`b${i}`} cx={0} cy={28 - i * 0.6} rx={30 + i * 9} ry={26 - i * 2.4} strokeOpacity={0.22 + i * 0.05} />)}
      <line x1={-110} y1={0} x2={110} y2={0} strokeOpacity={0.35} />
      <circle cx={younger ? -40 : 40} cy={0} r={11} className="lime-halo" stroke="none" />
      <circle cx={younger ? -40 : 40} cy={0} r={5} className="lime-dot" stroke="none" />
    </svg>
  );
}
