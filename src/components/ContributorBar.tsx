import { clamp } from "../lib/format";

export interface ContributorBarProps {
  label: string;
  /** 0..max */
  value: number;
  max?: number;
  /** Text at right, defaults to "<value>%". */
  valueText?: string;
  color?: string;
}

export function ContributorBar({ label, value, max = 100, valueText, color = "var(--sleep)" }: ContributorBarProps) {
  const pct = clamp((value / max) * 100, 0, 100);
  return (
    <div className="bar-row">
      <span className="bar-label">{label}</span>
      <span className="bar-val num">{valueText ?? `${Math.round(value)}%`}</span>
      <div className="bar-track" role="progressbar" aria-valuenow={Math.round(value)} aria-valuemax={max} aria-label={label}>
        <div className="bar-fill" style={{ width: `${pct}%`, background: color }} />
      </div>
    </div>
  );
}
