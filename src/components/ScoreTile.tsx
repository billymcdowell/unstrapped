import type { ReactNode } from "react";

export type Tone = "green" | "yellow" | "coral" | "dusk" | "ochre" | "rose";

export interface ScoreTileProps {
  tone: Tone;
  label: string;
  /** Pre-formatted big number; "—" when unavailable. */
  value: ReactNode;
  unit?: string;
  caption?: ReactNode;
  /** Line art under the number (usually a bare Dial). */
  children?: ReactNode;
  onClick?: () => void;
  className?: string;
}

/** Gradient tile with a large light number, white line art and an arrow when it navigates somewhere. */
export function ScoreTile({ tone, label, value, unit, caption, children, onClick, className = "" }: ScoreTileProps) {
  const inner = (
    <>
      <span className="score-label">{label}</span>
      {onClick && <span className="card-arrow" aria-hidden="true">↗</span>}
      <span className="score-value num">{value}{unit && <span className="score-unit">{unit}</span>}</span>
      <span className="score-caption">{caption}</span>
      {children && <div className="score-art">{children}</div>}
    </>
  );
  const cls = `card tone tone-${tone} score-tile ${className}`;
  return onClick ? (
    <button className={`${cls} clickable`} onClick={onClick}>{inner}</button>
  ) : (
    <section className={cls}>{inner}</section>
  );
}
