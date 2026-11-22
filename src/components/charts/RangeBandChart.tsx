import { useMemo } from "react";
import { useColorScheme } from "../../lib/hooks";
import { chartTheme } from "./theme";
import { EChart } from "./EChart";

export interface RangeBandPoint { label: string; value: number | null; lo: number | null; hi: number | null }

/** Line of daily values over a shaded "typical range" band; points outside the band are highlighted. */
export function RangeBandChart({ points, unit = "", height = 250, digits = 1 }: { points: RangeBandPoint[]; unit?: string; height?: number; digits?: number }) {
  const scheme = useColorScheme();
  const option = useMemo(() => {
    const th = chartTheme();
    const out = (p: RangeBandPoint) => p.value !== null && p.lo !== null && p.hi !== null && (p.value < p.lo || p.value > p.hi);
    const warn = th.rec.red;
    return {
      grid: { left: 44, right: 12, top: 14, bottom: 26 },
      tooltip: {
        trigger: "axis",
        formatter: (ps: { dataIndex: number }[]) => {
          const p = points[ps[0].dataIndex];
          const v = p.value === null ? "No reading" : `${p.value.toFixed(digits)} ${unit}${out(p) ? " (outside range)" : ""}`;
          const r = p.lo !== null && p.hi !== null ? `<br/>Typical ${p.lo.toFixed(digits)} to ${p.hi.toFixed(digits)}` : "";
          return `${p.label}<br/>${v}${r}`;
        },
      },
      xAxis: { type: "category", data: points.map((p) => p.label), axisTick: { show: false }, axisLine: { lineStyle: { color: th.grid } }, axisLabel: { color: th.text, hideOverlap: true } },
      yAxis: { type: "value", scale: true, axisLabel: { color: th.text }, splitLine: { lineStyle: { color: th.grid } } },
      series: [
        { name: "lo", type: "line", stack: "band", symbol: "none", silent: true, lineStyle: { opacity: 0 }, data: points.map((p) => p.lo) },
        { name: "range", type: "line", stack: "band", symbol: "none", silent: true, lineStyle: { opacity: 0 }, areaStyle: { color: th.accent, opacity: 0.14 }, data: points.map((p) => (p.lo !== null && p.hi !== null ? p.hi - p.lo : null)) },
        {
          name: "value", type: "line", smooth: 0.15, connectNulls: false, symbol: "circle", symbolSize: 5,
          lineStyle: { color: th.accent, width: 2 }, itemStyle: { color: th.accent },
          data: points.map((p) => (out(p) ? { value: p.value, symbolSize: 11, itemStyle: { color: warn, borderColor: "#fff", borderWidth: 1.5 } } : p.value)),
        },
      ],
    };
  }, [points, unit, digits, scheme]);
  return <EChart option={option} height={height} ariaLabel="30-day trend with typical range" />;
}
