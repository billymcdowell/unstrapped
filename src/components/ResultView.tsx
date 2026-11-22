import type { ReactNode } from "react";
import type { Result } from "../api/types";
import { EmptyState } from "./EmptyState";

/** Renders `children(data)` for ok results, an EmptyState for calibrating / noData. */
export function ResultView<T>({ result, children }: { result: Result<T> | null | undefined; children: (d: T) => ReactNode }) {
  if (!result) return <div className="empty faint">Loading…</div>;
  if (result.state === "ok") return <>{children(result.data)}</>;
  if (result.state === "calibrating") {
    const n = result.nightsRemaining;
    return <EmptyState title={`Calibrating — ${n} more night${n === 1 ? "" : "s"}`} detail={result.message} />;
  }
  return <EmptyState title="No data" detail={result.reason} gaps={result.gaps} />;
}
