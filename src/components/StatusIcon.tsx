import type { StatusLevel } from "../api/types";

const LABEL: Record<StatusLevel, string> = { ok: "In range", warn: "Slightly outside range", alert: "Well outside range", unavailable: "No reading" };

/** green check = in range, orange = slightly outside, red = well outside. */
export function StatusIcon({ level, size = 18 }: { level: StatusLevel; size?: number }) {
  const color = level === "ok" ? "var(--ok)" : level === "warn" ? "var(--warn)" : level === "alert" ? "var(--alert)" : "var(--text-3)";
  return (
    <svg className="status-icon" width={size} height={size} viewBox="0 0 20 20" role="img" aria-label={LABEL[level]}>
      <circle cx="10" cy="10" r="9" fill={color} opacity={level === "unavailable" ? 0.3 : 1} />
      {level === "ok" && <path d="M5.8 10.4l2.9 2.9 5.5-5.8" fill="none" stroke="#fff" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />}
      {(level === "warn" || level === "alert") && (
        <>
          <path d="M10 5.5v5.6" stroke="#fff" strokeWidth="2" strokeLinecap="round" />
          <circle cx="10" cy="14.2" r="1.15" fill="#fff" />
        </>
      )}
    </svg>
  );
}
