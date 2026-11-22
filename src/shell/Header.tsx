import { useNav, useStrap, type ScreenId } from "../lib/app-context";
import { addDays, formatDayLong, fmtTime, today } from "../lib/date";

const DAY_SCREENS: ScreenId[] = ["home", "sleep", "recovery", "strain", "health", "stress", "journal"];
const TITLES: Record<ScreenId, string> = {
  home: "Home", sleep: "Sleep", recovery: "Recovery", strain: "Strain", health: "Health Monitor", stress: "Stress Monitor", trends: "Trends",
  journal: "Journal", assessments: "Assessments", healthspan: "Healthspan", data: "Data", strap: "Strap",
};

export function Header() {
  const { screen, day, setDay } = useNav();
  const { status, progress } = useStrap();
  const isDay = DAY_SCREENS.includes(screen);
  const t = today();
  return (
    <header className="header" data-tauri-drag-region>
      {isDay ? (
        <div className="daynav">
          <button className="icon-btn" aria-label="Previous day" onClick={() => setDay(addDays(day, -1))}>◀</button>
          <span className="day-label">{formatDayLong(day)}</span>
          <button className="icon-btn" aria-label="Next day" disabled={day >= t} onClick={() => setDay(addDays(day, 1))}>▶</button>
          {day !== t && <button className="btn sm" onClick={() => setDay(t)}>Today</button>}
        </div>
      ) : (
        <span className="header-title" data-tauri-drag-region>{TITLES[screen]}</span>
      )}
      <span className="spacer" data-tauri-drag-region />
      <div className="sync-pill" aria-live="polite">
        {progress && progress.phase !== "done" ? (
          <>
            <span>{progress.label}</span>
            <span className="sync-bar"><i style={{ width: `${Math.round(progress.fraction * 100)}%` }} /></span>
          </>
        ) : status?.lastSyncMs ? (
          <span>Synced {fmtTime(status.lastSyncMs)} <span style={{ color: "var(--ok)" }}>✓</span></span>
        ) : (
          <span className="faint">Not synced yet</span>
        )}
      </div>
    </header>
  );
}
