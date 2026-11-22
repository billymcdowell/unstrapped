import { useNav, useStrap, type ScreenId } from "../lib/app-context";

const ITEMS: { id: ScreenId; label: string }[] = [
  { id: "home", label: "Home" }, { id: "sleep", label: "Sleep" }, { id: "recovery", label: "Recovery" }, { id: "strain", label: "Strain" },
  { id: "health", label: "Health" }, { id: "stress", label: "Stress" }, { id: "trends", label: "Trends" }, { id: "journal", label: "Journal" },
  { id: "assessments", label: "Assessments" }, { id: "healthspan", label: "Healthspan" },
];
const COLORS: Partial<Record<ScreenId, string>> = { sleep: "var(--sleep)", recovery: "var(--rec-green)", strain: "var(--strain)", stress: "var(--stress)" };

export function Sidebar() {
  const { screen, go } = useNav();
  const { status } = useStrap();
  const connected = status?.state === "connected" || status?.state === "syncing";
  const busy = status?.state === "scanning" || status?.state === "connecting";
  const batt = status?.batteryPct ?? null;
  return (
    <aside className="sidebar" aria-label="Navigation">
      <div className="sidebar-drag" data-tauri-drag-region />
      <div className="brand"><span className="brand-mark" />Unstrapped</div>
      <nav className="nav">
        {ITEMS.map((i) => (
          <button key={i.id} className="nav-item" aria-current={screen === i.id ? "page" : undefined} onClick={() => go(i.id)}>
            <span className="nav-icon" style={COLORS[i.id] ? { background: COLORS[i.id] } : undefined} />{i.label}
          </button>
        ))}
        <div className="nav-sep" />
        <button className="nav-item" aria-current={screen === "data" ? "page" : undefined} onClick={() => go("data")}>
          <span className="nav-icon" />Data
        </button>
      </nav>
      <button className="strap-chip" onClick={() => go("strap")} aria-label="Strap connection">
        <span className="strap-chip-row">
          <span className={`dot ${connected ? "on" : busy ? "busy" : status?.state === "error" ? "err" : ""}`} />
          Strap
          <span className="spacer" />
          {batt !== null && (
            <>
              <span className={`battery ${batt < 20 ? "low" : ""}`}><i style={{ width: `${batt}%` }} /></span>
              <span className="num muted">{batt}%</span>
            </>
          )}
        </span>
        <span className="faint" style={{ fontSize: 11 }}>
          {connected ? (status?.charging ? "Charging" : status?.onWrist === false ? "Connected · off wrist" : "Connected") : busy ? "Connecting…" : status?.state === "error" ? "Error" : "Not connected"}
        </span>
      </button>
    </aside>
  );
}
