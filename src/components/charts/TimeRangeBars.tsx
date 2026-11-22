import { useMemo } from "react";
import { useColorScheme } from "../../lib/hooks";
import { fmtTime } from "../../lib/date";
import { resolveColor } from "../../lib/format";
import { chartTheme } from "./theme";
import { EChart } from "./EChart";

/** minutes since 18:00 local, so a night 22:30 -> 06:45 is one continuous bar. */
const sinceEvening = (ms: number) => { const d = new Date(ms); const m = d.getHours() * 60 + d.getMinutes(); return (m - 18 * 60 + 1440) % 1440; };

/** Floating horizontal bars: bed time to wake time per night (sleep consistency). */
export function TimeRangeBars({ rows, color, height }: { rows: { label: string; startMs: number; endMs: number }[]; color?: string; height?: number }) {
  const scheme = useColorScheme();
  const h = height ?? 30 + rows.length * 20;
  const option = useMemo(() => {
    const th = chartTheme();
    const starts = rows.map((r) => sinceEvening(r.startMs));
    const durs = rows.map((r) => Math.round((r.endMs - r.startMs) / 60_000));
    return {
      grid: { left: 54, right: 14, top: 4, bottom: 24 },
      tooltip: { trigger: "axis", axisPointer: { type: "none" }, formatter: (p: { dataIndex: number }[]) => `${rows[p[0].dataIndex].label}<br/>${fmtTime(rows[p[0].dataIndex].startMs)} – ${fmtTime(rows[p[0].dataIndex].endMs)}` },
      xAxis: { type: "value", min: 0, max: 16 * 60, interval: 240, axisLabel: { color: th.text, formatter: (v: number) => fmtTime(new Date(2000, 0, 1, 18, 0).getTime() + v * 60_000) }, splitLine: { lineStyle: { color: th.grid } } },
      yAxis: { type: "category", inverse: true, data: rows.map((r) => r.label), axisTick: { show: false }, axisLine: { show: false }, axisLabel: { color: th.text, fontSize: 10 } },
      series: [
        { type: "bar", stack: "s", itemStyle: { color: "transparent" }, silent: true, data: starts },
        { type: "bar", stack: "s", barWidth: 10, itemStyle: { color: color ? resolveColor(color) : th.sleep, borderRadius: 4 }, data: durs },
      ],
    };
  }, [rows, color, scheme]);
  return <EChart option={option} height={h} ariaLabel="Bed and wake times" />;
}
