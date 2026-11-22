import { useState } from "react";
import type { ActivitySummary, Day } from "../api/types";
import * as api from "../api/client";
import { dayStartMs, fmtTime } from "../lib/date";
import { fmt } from "../lib/format";
import { useNav } from "../lib/app-context";

const KINDS = ["Run", "Walk", "Cycling", "Strength", "Yoga", "Swim", "Other"];

export function ActivityRow({ a, onOpen }: { a: ActivitySummary; onOpen: (id: string) => void }) {
  return (
    <button className="list-row" onClick={() => onOpen(a.id)}>
      <span className="act-pill" />
      <span className="act-kind">{a.kind}{a.manual && <span className="faint"> · manual</span>}</span>
      <span className="muted num">{fmtTime(a.startMs)}–{fmtTime(a.endMs)}</span>
      <span className="spacer" />
      <span className="num"><b>{fmt(a.strain, 1)}</b> <span className="faint">strain</span></span>
      <span className="num muted">{fmt(a.avgHr)} bpm avg</span>
      <span className="num muted">{fmt(a.calories)} kcal</span>
    </button>
  );
}

/** List of activity rows (auto-detected and manual) with an empty message. */
export function ActivityList({ activities, onOpen }: { activities: ActivitySummary[]; onOpen: (id: string) => void }) {
  if (!activities.length) return <div className="empty faint" style={{ minHeight: 60 }}>No activities detected yet</div>;
  return <div className="list">{activities.map((a) => <ActivityRow key={a.id} a={a} onOpen={onOpen} />)}</div>;
}

/** Inline manual-activity form. Calls add_activity, then refreshes the screen. */
export function AddActivity({ day }: { day: Day }) {
  const nav = useNav();
  const [open, setOpen] = useState(false);
  const [kind, setKind] = useState("Run");
  const [start, setStart] = useState("18:00");
  const [mins, setMins] = useState(45);
  const [busy, setBusy] = useState(false);

  if (!open) return <button className="btn sm" onClick={() => setOpen(true)}>+ Add activity</button>;

  const save = async () => {
    const [h, m] = start.split(":").map(Number);
    const startMs = dayStartMs(day) + (h * 60 + m) * 60_000;
    setBusy(true);
    try { await api.addActivity(kind, startMs, startMs + mins * 60_000); nav.refresh(); setOpen(false); } finally { setBusy(false); }
  };
  return (
    <div className="row wrap">
      <div className="field"><label>Type</label><select value={kind} onChange={(e) => setKind(e.target.value)}>{KINDS.map((k) => <option key={k}>{k}</option>)}</select></div>
      <div className="field"><label>Start</label><input type="time" value={start} onChange={(e) => setStart(e.target.value)} /></div>
      <div className="field"><label>Minutes</label><input type="number" min={5} max={600} value={mins} onChange={(e) => setMins(Number(e.target.value))} style={{ width: 80 }} /></div>
      <button className="btn primary sm" disabled={busy || !start || mins < 1} onClick={save} style={{ alignSelf: "flex-end" }}>Save</button>
      <button className="btn sm" onClick={() => setOpen(false)} style={{ alignSelf: "flex-end" }}>Cancel</button>
    </div>
  );
}
