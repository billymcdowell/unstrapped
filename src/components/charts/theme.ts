import { cssVar } from "../../lib/format";

/** Concrete colours for canvas charts; call during render with useColorScheme() so it refreshes on theme change. */
export function chartTheme() {
  return {
    text: cssVar("--chart-axis"),
    textStrong: cssVar("--text"),
    grid: cssVar("--chart-grid"),
    track: cssVar("--band-track"),
    card: cssVar("--card"),
    accent: cssVar("--accent"),
    stages: { awake: cssVar("--stage-awake"), light: cssVar("--stage-light"), deep: cssVar("--stage-deep"), rem: cssVar("--stage-rem") },
    zones: [1, 2, 3, 4, 5].map((i) => cssVar(`--zone-${i}`)),
    rec: { red: cssVar("--rec-red"), yellow: cssVar("--rec-yellow"), green: cssVar("--rec-green") },
    sleep: cssVar("--sleep"), strain: cssVar("--strain"), stress: cssVar("--stress"),
    font: cssVar("--font", "system-ui"),
  };
}
export type ChartTheme = ReturnType<typeof chartTheme>;

const rgb = (c: string): number[] | null => {
  const m = /^#([0-9a-f]{6})$/i.exec(c.trim());
  return m ? [0, 2, 4].map((i) => parseInt(m[1].slice(i, i + 2), 16) / 255) : null;
};
const lum = (v: number[]) => {
  const [r, g, b] = v.map((x) => (x <= 0.04045 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const ratio = (a: number, b: number) => (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);

/** Dark ink or white, whichever has more contrast on `fill` drawn at `opacity` over the card. Keeps in-bar labels AA. */
export function readableOn(fill: string, opacity = 1): string {
  const f = rgb(fill), card = rgb(cssVar("--card", "#ffffff"));
  if (!f || !card) return cssVar("--text", "#1b1b19");
  const bg = lum(f.map((x, i) => x * opacity + card[i] * (1 - opacity)));
  const INK = "#1b1b19";
  return ratio(1, bg) >= ratio(lum(rgb(INK)!), bg) ? "#ffffff" : INK;
}

/** `fill` drawn at `opacity` over the card, as an opaque hex (so labels on it don't inherit the opacity). */
export function overCard(fill: string, opacity: number): string {
  const f = rgb(fill), card = rgb(cssVar("--card", "#ffffff"));
  if (!f || !card) return fill;
  return "#" + f.map((x, i) => Math.round((x * opacity + card[i] * (1 - opacity)) * 255).toString(16).padStart(2, "0")).join("");
}
