import { useEffect, useRef, useState } from "react";
import { Segmented } from "./Segmented";

interface Phase { label: string; secs: number; scale: number }
const PATTERNS: Record<string, { name: string; phases: Phase[] }> = {
  resonant: { name: "Calm 5.5 s", phases: [{ label: "Breathe in", secs: 5.5, scale: 1 }, { label: "Breathe out", secs: 5.5, scale: 0.55 }] },
  box: { name: "Box 4-4-4-4", phases: [{ label: "Breathe in", secs: 4, scale: 1 }, { label: "Hold", secs: 4, scale: 1 }, { label: "Breathe out", secs: 4, scale: 0.55 }, { label: "Hold", secs: 4, scale: 0.55 }] },
  relax: { name: "Relax 4-7-8", phases: [{ label: "Breathe in", secs: 4, scale: 1 }, { label: "Hold", secs: 7, scale: 1 }, { label: "Breathe out", secs: 8, scale: 0.55 }] },
};
type PatternId = keyof typeof PATTERNS;

const reducedMotion = () => typeof window !== "undefined" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;

/** Guided breathing timer. The circle grows on inhale and shrinks on exhale; with reduced motion it stays still and the text carries the cue. */
export function BreathingTimer() {
  const [pattern, setPattern] = useState<PatternId>("resonant");
  const [minutes, setMinutes] = useState("3");
  const [elapsed, setElapsed] = useState<number | null>(null); // seconds; null = idle
  const startRef = useRef(0);
  const total = Number(minutes) * 60;
  const running = elapsed !== null && elapsed < total;
  const still = reducedMotion();

  useEffect(() => {
    if (!running) return;
    const id = window.setInterval(() => {
      const e = (performance.now() - startRef.current) / 1000;
      setElapsed(Math.min(e, total));
    }, 100);
    return () => window.clearInterval(id);
  }, [running, total]);

  const phases = PATTERNS[pattern].phases;
  const cycle = phases.reduce((s, p) => s + p.secs, 0);
  let phase: Phase | null = null;
  let left = 0;
  if (running) {
    let t = (elapsed as number) % cycle;
    for (const p of phases) { if (t < p.secs) { phase = p; left = p.secs - t; break; } t -= p.secs; }
  }
  const done = elapsed !== null && elapsed >= total;
  const remaining = elapsed === null ? total : Math.max(0, total - elapsed);
  const mm = Math.floor(remaining / 60), ss = Math.ceil(remaining % 60);
  const scale = still ? 0.8 : phase ? phase.scale : 0.55;
  const label = done ? "Done" : phase ? phase.label : "Ready";

  return (
    <div className="breath">
      <div className="breath-stage">
        <div
          className="breath-circle"
          style={{ transform: `scale(${scale})`, transitionDuration: phase && !still ? `${phase.secs}s` : "0.4s" }}
        />
        <div className="breath-text" role="status" aria-live="polite">
          <span className="breath-phase">{label}</span>
          {running && <span className="faint num">{Math.ceil(left)}</span>}
        </div>
      </div>
      <div className="col" style={{ alignItems: "center" }}>
        <span className="num muted">{mm}:{String(ss === 60 ? 59 : ss).padStart(2, "0")} left</span>
        <div className="row wrap" style={{ justifyContent: "center" }}>
          <Segmented ariaLabel="Breathing pattern" value={pattern} onChange={(v) => !running && setPattern(v)} options={(Object.keys(PATTERNS) as PatternId[]).map((k) => ({ value: k, label: PATTERNS[k].name }))} />
          <Segmented ariaLabel="Duration" value={minutes} onChange={(v) => !running && setMinutes(v)} options={[{ value: "1", label: "1 min" }, { value: "3", label: "3 min" }, { value: "5", label: "5 min" }]} />
        </div>
        {running ? (
          <button className="btn" onClick={() => setElapsed(null)}>Stop</button>
        ) : (
          <button className="btn primary" onClick={() => { startRef.current = performance.now(); setElapsed(0); }}>{done ? "Go again" : "Start"}</button>
        )}
        {still && <span className="faint">Reduced motion is on, so the circle stays still. Follow the words.</span>}
      </div>
    </div>
  );
}
