import type { Band, StrainBand, StressBand } from "../api/types";

export const recoveryBand = (s: number): Band => (s >= 67 ? "green" : s >= 34 ? "yellow" : "red");
export const strainBand = (s: number): StrainBand => (s >= 18 ? "allOut" : s >= 14 ? "high" : s >= 10 ? "moderate" : "light");
export const strainBandLabel: Record<StrainBand, string> = { light: "Light", moderate: "Moderate", high: "High", allOut: "All out" };
export const stressBandLabel: Record<StressBand, string> = { low: "Low", medium: "Medium", high: "High" };

export const fmt = (n: number | null | undefined, digits = 0): string =>
  n === null || n === undefined || Number.isNaN(n) ? "—" : n.toLocaleString(undefined, { minimumFractionDigits: digits, maximumFractionDigits: digits });

export const bandVar = (b: Band | null | undefined): string =>
  b ? `var(--${b === "red" ? "rec-red" : b === "yellow" ? "rec-yellow" : "rec-green"})` : "var(--text-3)";

export const clamp = (n: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, n));

export const ZONE_COLORS = ["var(--zone-1)", "var(--zone-2)", "var(--zone-3)", "var(--zone-4)", "var(--zone-5)"];

/** Read a CSS custom property's resolved value (for canvas charts). */
export function cssVar(name: string, fallback = "#888"): string {
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return v || fallback;
}

/** Resolve "var(--x)" (or a plain colour) to a concrete colour string for canvas drawing. */
export function resolveColor(c: string): string {
  const m = /^var\((--[\w-]+)\)$/.exec(c.trim());
  return m ? cssVar(m[1]) : c;
}

/** Zone colour bands for a heart-rate line; `bounds` = lower bpm of zones 2..5. */
export function hrZones(bounds: [number, number, number, number]) {
  const edges = [0, ...bounds, 1000];
  return ZONE_COLORS.map((color, i) => ({ from: edges[i], to: edges[i + 1], color }));
}
