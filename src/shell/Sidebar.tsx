import { useNav, useStrap, type ScreenId } from "../lib/app-context";
import { BrandMark, NavIcon } from "./icons";

const GROUPS: { label?: string; items: { id: ScreenId; label: string }[] }[] = [
  { items: [{ id: "home", label: "Home" }] },
  { label: "Today", items: [{ id: "sleep", label: "Sleep" }, { id: "recovery", label: "Recovery" }, { id: "strain", label: "Strain" }, { id: "health", label: "Health" }, { id: "stress", label: "Stress" }] },
  { label: "Insights", items: [{ id: "trends", label: "Trends" }, { id: "journal", label: "Journal" }, { id: "assessments", label: "Assessments" }, { id: "healthspan", label: "Healthspan" }] },
  { label: "Library", items: [{ id: "data", label: "Data" }] },
];

export function Sidebar() {
  const { screen, go } = useNav();
  const { status } = useStrap();
  const connected = status?.state === "connected" || status?.state === "syncing";
  const busy = status?.state === "scanning" || status?.state === "connecting";
  const batt = status?.batteryPct ?? null;
  return (
    <aside className="sidebar" aria-label="Navigation">
      <div className="sidebar-drag" data-tauri-drag-region />
      <div className="brand"><span className="brand-mark"><BrandMark /></span>Unstrapped</div>
      <nav className="nav">
        {GROUPS.map((g, gi) => (
          <div key={gi} className="col" style={{ gap: 1 }}>
            {g.label && <div className="nav-group">{g.label}</div>}
            {g.items.map((i) => (
              <button key={i.id} className="nav-item" aria-current={screen === i.id ? "page" : undefined} onClick={() => go(i.id)}>
                <NavIcon id={i.id} />{i.label}
              </button>
            ))}
          </div>
        ))}
      </nav>
      <button className="strap-chip" onClick={() => go("strap")} aria-label="Strap connection" aria-current={screen === "strap" ? "page" : undefined}>
        <span className="strap-chip-row">
          <span className={`dot ${connected ? "on" : busy ? "busy" : status?.state === "error" ? "err" : ""}`} />
          Strap
          <span className="spacer" />
          {batt !== null && (
            <>
              <span className={`battery ${batt < 20 ? "low" : ""}`}><i style={{ width: `${batt}%` }} /></span>
              <span className="num muted" style={{ fontSize: 12 }}>{batt}%</span>
            </>
          )}
        </span>
        <span className="faint" style={{ fontSize: 12 }}>
          {connected ? (status?.charging ? "Charging" : status?.onWrist === false ? "Connected · off wrist" : "Connected") : busy ? "Connecting…" : status?.state === "error" ? "Error" : "Not connected"}
        </span>
      </button>
    </aside>
  );
}
