import { clamp } from "../lib/format";

export interface DialProps {
  /** Current value; null renders an empty ring (calibrating / no data). */
  value: number | null;
  /** 100 for Sleep/Recovery, 21 for Strain. */
  max?: number;
  /** Ring colour, CSS colour or var(). */
  color: string;
  /** Uppercase caption under the ring, e.g. "Recovery". */
  label: string;
  /** Text shown in the centre; defaults to rounded value (+ "%" when max is 100). */
  display?: string;
  /** Small text under the centre number. */
  sub?: string;
  /** Lighter target-band arc drawn around the ring (Strain target). */
  target?: { lo: number; hi: number } | null;
  /** Line under the label, e.g. "target 14–18". */
  caption?: string;
  size?: number;
  onClick?: () => void;
}

/** SVG ring dial drawn with stroke-dasharray. */
export function Dial({ value, max = 100, color, label, display, sub, target, caption, size = 160, onClick }: DialProps) {
  const stroke = 14;
  const r = (size - stroke) / 2 - (target ? 7 : 0);
  const c = 2 * Math.PI * r;
  const frac = value === null ? 0 : clamp(value / max, 0, 1);
  const rt = r + stroke / 2 + 5;
  const ct = 2 * Math.PI * rt;
  const text = display ?? (value === null ? "—" : max === 100 ? `${Math.round(value)}%` : value.toFixed(1));
  const body = (
    <>
      <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} role="img" aria-label={`${label} ${text}`}>
        <g transform={`translate(${size / 2} ${size / 2}) rotate(-90)`}>
          <circle r={r} fill="none" stroke="var(--band-track)" strokeWidth={stroke} />
          {target && (
            <circle
              r={rt} fill="none" stroke={color} strokeOpacity={0.35} strokeWidth={5} strokeLinecap="round"
              strokeDasharray={`${clamp((target.hi - target.lo) / max, 0, 1) * ct} ${ct}`}
              transform={`rotate(${clamp(target.lo / max, 0, 1) * 360})`}
            />
          )}
          <circle
            className="dial-arc" r={r} fill="none" stroke={color} strokeWidth={stroke} strokeLinecap="round"
            strokeDasharray={`${frac * c} ${c}`} opacity={value === null ? 0 : 1}
          />
        </g>
        <text className="dial-value num" x="50%" y={sub ? "48%" : "52%"} textAnchor="middle" dominantBaseline="middle">{text}</text>
        {sub && <text className="dial-sub" x="50%" y="64%" textAnchor="middle">{sub}</text>}
      </svg>
      <span className="dial-label">{label}</span>
      <span className="dial-caption">{caption ?? ""}</span>
    </>
  );
  return onClick ? (
    <button className="dial" style={{ background: "none", border: 0 }} onClick={onClick}>{body}</button>
  ) : (
    <div className="dial">{body}</div>
  );
}
