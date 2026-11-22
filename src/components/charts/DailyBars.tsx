import { useMemo } from "react";
import { useColorScheme } from "../../lib/hooks";
import { resolveColor } from "../../lib/format";
import { chartTheme } from "./theme";
import { EChart } from "./EChart";

export interface DailyBarPoint { label: string; value: number | null; color?: string }

/** Bars (or a line) per day/week with an optional shaded baseline band. Used for 7-day recovery and Trends. */
export function DailyBars({ points, unit = "", yMax, yMin, baseline, line, height = 220, color }: {
  points: DailyBarPoint[]; unit?: string; yMax?: number; yMin?: number; baseline?: { lo: number; hi: number } | null; line?: boolean; height?: number; color?: string;
}) {
  const scheme = useColorScheme();
  const option = useMemo(() => {
    const th = chartTheme();
    const c = color ?? th.accent;
    const mark = baseline ? { silent: true, itemStyle: { color: th.accent, opacity: 0.1 }, data: [[{ yAxis: baseline.lo }, { yAxis: baseline.hi }]] } : undefined;
    return {
      grid: { left: 40, right: 12, top: 12, bottom: 26 },
      tooltip: { trigger: "axis", formatter: (p: { axisValue: string; value: number | null }[]) => `${p[0].axisValue}<br/>${p[0].value === null || p[0].value === undefined ? "No data" : p[0].value + unit}` },
      xAxis: { type: "category", data: points.map((p) => p.label), axisTick: { show: false }, axisLine: { lineStyle: { color: th.grid } }, axisLabel: { color: th.text, hideOverlap: true } },
      yAxis: { type: "value", min: yMin, max: yMax, axisLabel: { color: th.text }, splitLine: { lineStyle: { color: th.grid } } },
      series: [line
        ? { type: "line", smooth: 0.2, symbol: "circle", symbolSize: 6, connectNulls: false, lineStyle: { color: c, width: 1.5 }, itemStyle: { color: c }, markArea: mark, data: points.map((p) => p.value) }
        : { type: "bar", barMaxWidth: 18, itemStyle: { borderRadius: [3, 3, 0, 0] }, markArea: mark, data: points.map((p) => ({ value: p.value, itemStyle: { color: p.color ? resolveColor(p.color) : c } })) }],
    };
  }, [points, unit, yMax, yMin, baseline, line, color, scheme]);
  return <EChart option={option} height={height} ariaLabel="Daily values" />;
}
