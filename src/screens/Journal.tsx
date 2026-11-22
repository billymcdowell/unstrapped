import { useEffect, useMemo, useState } from "react";
import * as api from "../api/client";
import type { Insight, JournalQuestion } from "../api/types";
import { Card, DivergingBars, EmptyState, Segmented } from "../components";
import { useNav } from "../lib/app-context";
import { useApi } from "../lib/hooks";
import { addDays, formatDayLong, parseDay, today } from "../lib/date";

type Tab = "checkin" | "insights";
type Outcome = "recoveryDelta" | "sleepDelta" | "hrvDelta";
const OUTCOMES: { value: Outcome; label: string; what: string }[] = [
  { value: "recoveryDelta", label: "Recovery", what: "next-day recovery" },
  { value: "sleepDelta", label: "Sleep performance", what: "next-day sleep performance" },
  { value: "hrvDelta", label: "HRV", what: "next-day HRV" },
];

export function Journal() {
  const [tab, setTab] = useState<Tab>("checkin");
  return (
    <div className="page">
      <div className="row wrap">
        <div>
          <h1 className="page-title">Journal</h1>
          <p className="page-sub">Log what you did, then see how it relates to how you recover.</p>
        </div>
        <span className="spacer" />
        <Segmented ariaLabel="Journal view" value={tab} onChange={setTab} options={[{ value: "checkin", label: "Check-in" }, { value: "insights", label: "Insights" }]} />
      </div>
      {tab === "checkin" ? <CheckIn /> : <Insights />}
    </div>
  );
}

// ------------------------------------------------------------ check-in

function CheckIn() {
  const nav = useNav();
  const day = addDays(nav.day, -1); // behaviours affect the next day's recovery, so ask about yesterday
  const { data, error } = useApi(() => api.getJournal(day), [day]);
  const [local, setLocal] = useState<Record<string, boolean | null>>({});
  const [failed, setFailed] = useState<string | null>(null);
  useEffect(() => { setLocal({}); }, [data, day]);

  const answer = async (q: JournalQuestion, a: boolean | null) => {
    setLocal((l) => ({ ...l, [q.id]: a }));
    setFailed(null);
    try { await api.setJournalAnswer(day, q.id, a); nav.refresh(); }
    catch (e) { setLocal((l) => { const n = { ...l }; delete n[q.id]; return n; }); setFailed(String((e as Error)?.message ?? e)); }
  };

  return (
    <>
      <Card title={`About ${formatDayLong(day).toLowerCase()}`}>
        {error && !data ? <EmptyState title="Couldn't load" detail={error} /> : !data ? <div className="empty faint">Loading…</div> : data.questions.length === 0 ? (
          <EmptyState title="No behaviours picked" detail="Choose some below to start your daily check-in." />
        ) : (
          <div className="list">
            {data.questions.map((q) => {
              const a = q.id in local ? local[q.id] : q.answer;
              return (
                <div key={q.id} className="list-row">
                  <span style={{ fontWeight: 550 }}>{q.label}</span>
                  <span className="faint">{q.category}</span>
                  <span className="spacer" />
                  <div className="seg" role="group" aria-label={q.label}>
                    <button role="checkbox" aria-checked={a === true} onClick={() => answer(q, a === true ? null : true)}>Yes</button>
                    <button role="checkbox" aria-checked={a === false} onClick={() => answer(q, a === false ? null : false)}>No</button>
                  </div>
                </div>
              );
            })}
          </div>
        )}
        {failed && <p className="error-text" style={{ marginTop: 8 }}>Couldn't save: {failed}</p>}
        <p className="faint" style={{ marginTop: 10 }}>Tap an answer again to clear it. Unanswered days are ignored.</p>
      </Card>
      <BehaviourPicker />
    </>
  );
}

function BehaviourPicker() {
  const nav = useNav();
  const { data } = useApi(() => api.getJournalLibrary(), []);
  const [open, setOpen] = useState(false);
  const [sel, setSel] = useState<Set<string> | null>(null);
  useEffect(() => { if (data) setSel(new Set(data.filter((l) => l.selected).map((l) => l.id))); }, [data]);
  const groups = useMemo(() => {
    const m = new Map<string, NonNullable<typeof data>>();
    data?.forEach((l) => m.set(l.category, [...(m.get(l.category) ?? []), l]));
    return [...m.entries()];
  }, [data]);

  if (!data || !sel) return null;
  const toggle = async (id: string) => {
    const next = new Set(sel);
    if (next.has(id)) next.delete(id); else next.add(id);
    setSel(next);
    try { await api.setJournalBehaviors([...next]); nav.refresh(); } catch { setSel(sel); }
  };
  return (
    <Card title="Your behaviours" action={{ label: open ? "Done" : "Edit", onClick: () => setOpen(!open) }}>
      {!open ? (
        <p className="muted">{[...sel].length ? data.filter((l) => sel.has(l.id)).map((l) => l.label).join(" · ") : "None selected."}</p>
      ) : (
        <div className="col" style={{ gap: 14 }}>
          {groups.map(([cat, items]) => (
            <div key={cat}>
              <div className="card-title" style={{ marginBottom: 6 }}>{cat}</div>
              <div className="row wrap" style={{ gap: 8 }}>
                {items.map((l) => (
                  <button key={l.id} className="chip" aria-pressed={sel.has(l.id)} onClick={() => toggle(l.id)}>{l.label}</button>
                ))}
              </div>
            </div>
          ))}
        </div>
      )}
    </Card>
  );
}

// ------------------------------------------------------------ insights

function Insights() {
  const { data, error } = useApi(() => api.getInsights(), []);
  const [outcome, setOutcome] = useState<Outcome>("recoveryDelta");
  const meta = OUTCOMES.find((o) => o.value === outcome)!;
  const { ready, waiting } = useMemo(() => {
    const ready: Insight[] = [], waiting: Insight[] = [];
    (data ?? []).forEach((i) => (i.state === "ok" && i[outcome] !== null ? ready : waiting).push(i));
    ready.sort((a, b) => (b[outcome] as number) - (a[outcome] as number));
    return { ready, waiting };
  }, [data, outcome]);

  if (error && !data) return <EmptyState title="Couldn't load" detail={error} />;
  if (!data) return <div className="empty faint">Loading…</div>;
  return (
    <>
      <Card title={`How behaviours relate to ${meta.what}`}>
        <div style={{ marginBottom: 12 }}><Segmented ariaLabel="Outcome" value={outcome} onChange={setOutcome} options={OUTCOMES.map((o) => ({ value: o.value, label: o.label }))} /></div>
        {ready.length ? (
          <>
            <DivergingBars rows={ready.map((i) => ({ key: i.behavior, label: i.label, value: i[outcome] as number, sub: `${i.yesDays} yes / ${i.noDays} no` }))} />
            <div className="legend" style={{ marginTop: 10 }}>
              <span><i style={{ background: "var(--ok)" }} />Helped</span>
              <span><i style={{ background: "var(--alert)" }} />Hurt</span>
              <span>Average {meta.what}, days with a "yes" compared with days with a "no", last 90 days.</span>
            </div>
          </>
        ) : <EmptyState title="Nothing to show yet" detail="A behaviour needs at least 5 yes and 5 no answers in 90 days before it appears here." />}
      </Card>
      {waiting.length > 0 && (
        <Card title="Not enough data yet">
          <div className="list">
            {waiting.map((i) => (
              <div key={i.behavior} className="list-row">
                <span style={{ fontWeight: 550 }}>{i.label}</span>
                <span className="spacer" />
                <span className="muted num">Not enough data yet ({i.yesDays} yes / {i.noDays} no)</span>
              </div>
            ))}
          </div>
          <p className="faint" style={{ marginTop: 8 }}>Needs at least 5 yes and 5 no.</p>
        </Card>
      )}
      <p className="faint">Correlation is not cause. Insights since {parseDay(addDays(today(), -90)).toLocaleDateString(undefined, { day: "numeric", month: "short" })}.</p>
    </>
  );
}
