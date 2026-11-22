import type { Gap } from "../api/types";
import { fmtTime } from "../lib/date";

export interface EmptyStateProps {
  title: string;
  detail?: string;
  /** e.g. "Strap was off 02:10-04:30" lines are generated from these. */
  gaps?: Gap[];
}

/** Used for "Calibrating - 2 more nights" and "No data" states. Never show a fake zero instead. */
export function EmptyState({ title, detail, gaps = [] }: EmptyStateProps) {
  const real = gaps.filter((g) => g.endMs - g.startMs < 23 * 3_600_000);
  return (
    <div className="empty">
      <div className="empty-title">{title}</div>
      {detail && <div>{detail}</div>}
      {real.map((g, i) => (
        <div key={i} className="empty-gap num">{g.reason} {fmtTime(g.startMs)}–{fmtTime(g.endMs)}</div>
      ))}
    </div>
  );
}
