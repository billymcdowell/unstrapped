import type { ReactNode } from "react";

export interface StatTileProps {
  label: string;
  /** Pre-formatted value; use "—" for unavailable. */
  value: ReactNode;
  unit?: string;
  sub?: ReactNode;
  color?: string;
}

export function StatTile({ label, value, unit, sub, color }: StatTileProps) {
  return (
    <div className="stat">
      <span className="stat-label">{label}</span>
      <span className="stat-value num" style={color ? { color } : undefined}>
        {value}
        {unit && <span className="stat-unit">{unit}</span>}
      </span>
      {sub && <span className="stat-sub">{sub}</span>}
    </div>
  );
}
