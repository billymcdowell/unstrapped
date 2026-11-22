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
