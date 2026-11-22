import { clamp } from "../lib/format";

export interface DialProps {
  /** Current value; null renders an empty ring (calibrating / no data). */
  value: number | null;
  /** 100 for Sleep/Recovery, 21 for Strain. */
  max?: number;
  /** Arc colour, CSS colour or var(). Use "var(--text)" on tone tiles. */
  color: string;
  /** Caption under the ring, e.g. "Recovery". */
  label?: string;
  /** Text shown in the centre; defaults to rounded value (+ "%" when max is 100). */
  display?: string;
  /** Small text under the centre number. */
  sub?: string;
  /** Lighter target-band arc drawn around the ring (Strain target). */
  target?: { lo: number; hi: number } | null;
  /** Line under the label, e.g. "target 14–18". */
  caption?: string;
  size?: number;
  /** Draw the ring only (the number is shown elsewhere, e.g. on a score tile). */
  bare?: boolean;
  onClick?: () => void;
}

/** Line-art ring: faint concentric guides, a thin arc, and a lime marker at the current value. */
export function Dial({ value, max = 100, color, label, display, sub, target, caption, size = 160, bare, onClick }: DialProps) {
  const r = size / 2 - 12;
  const c = 2 * Math.PI * r;
  const frac = value === null ? 0 : clamp(value / max, 0, 1);
  const rt = r + 8;
  const ct = 2 * Math.PI * rt;
  const text = display ?? (value === null ? "—" : max === 100 ? `${Math.round(value)}%` : value.toFixed(1));
  const a = frac * 2 * Math.PI;
  const mx = Math.cos(a) * r, my = Math.sin(a) * r; // marker, in the rotated frame
  const guides = [0.86, 0.72, 0.58, 0.44];
  const body = (
    <>
      <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} role="img" aria-label={`${label ?? "Score"} ${text}`}>
        <g transform={`translate(${size / 2} ${size / 2}) rotate(-90)`}>
          {guides.map((g) => <circle key={g} className="dial-guide" r={r * g} fill="none" stroke="var(--text)" strokeOpacity={0.09} strokeWidth={0.75} />)}
          <circle className="dial-track" r={r} fill="none" stroke="var(--band-track)" strokeWidth={1.25} />
          {target && (
            <circle
              r={rt} fill="none" stroke={color} strokeOpacity={0.5} strokeWidth={2} strokeLinecap="round"
              strokeDasharray={`${clamp((target.hi - target.lo) / max, 0, 1) * ct} ${ct}`}
              transform={`rotate(${clamp(target.lo / max, 0, 1) * 360})`}
            />
          )}
          <circle
            className="dial-arc" r={r} fill="none" stroke={color} strokeWidth={3} strokeLinecap="round"
            strokeDasharray={`${frac * c} ${c}`} opacity={value === null ? 0 : 1}
          />
          {value !== null && (
            <>
              <circle className="lime-halo" cx={mx} cy={my} r={11} />
              <circle className="lime-dot" cx={mx} cy={my} r={5} />
            </>
          )}
        </g>
        {!bare && <text className="dial-value num" x="50%" y={sub ? "47%" : "51%"} textAnchor="middle" dominantBaseline="middle">{text}</text>}
        {!bare && sub && <text className="dial-sub" x="50%" y="62%" textAnchor="middle">{sub}</text>}
      </svg>
      {label && !bare && <span className="dial-label">{label}</span>}
      {caption !== undefined && !bare && <span className="dial-caption">{caption}</span>}
    </>
  );
  return onClick ? (
    <button className="dial" style={{ background: "none", border: 0 }} onClick={onClick}>{body}</button>
  ) : (
    <div className="dial">{body}</div>
  );
}
