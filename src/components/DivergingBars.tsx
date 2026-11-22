export interface DivergingRow { key: string; label: string; value: number; sub?: string }

const MINUS = "−";
export const signed = (n: number, unit = "%") => `${n > 0 ? "+" : n < 0 ? MINUS : ""}${Math.abs(Math.round(n * 10) / 10)}${unit}`;

/** Zero-centred horizontal bars: green = helped (positive), red = hurt (negative). Plain DOM, so it follows light/dark tokens. */
export function DivergingBars({ rows, unit = "%", max }: { rows: DivergingRow[]; unit?: string; max?: number }) {
  const m = max ?? Math.max(5, ...rows.map((r) => Math.abs(r.value)));
  return (
    <div className="div-bars" role="list">
      {rows.map((r) => {
        const w = Math.min(50, (Math.abs(r.value) / m) * 50);
        const good = r.value >= 0;
        return (
          <div className="div-row" role="listitem" key={r.key}>
            <span className="div-label">{r.label}{r.sub && <span className="faint div-sub"> {r.sub}</span>}</span>
            <div className="div-track" aria-hidden="true">
              <i className="div-zero" />
              <i className="div-bar" style={{ width: `${w}%`, [good ? "left" : "right"]: "50%", background: good ? "var(--ok)" : "var(--alert)", borderRadius: good ? "0 4px 4px 0" : "4px 0 0 4px" }} />
            </div>
            <span className="div-val num" style={{ color: good ? "var(--ok)" : "var(--alert)" }}>{signed(r.value, unit)}</span>
          </div>
        );
      })}
    </div>
  );
}
