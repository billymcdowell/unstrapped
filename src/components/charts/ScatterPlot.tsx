import { useMemo } from "react";
import { useColorScheme } from "../../lib/hooks";
import { resolveColor } from "../../lib/format";
import { chartTheme } from "./theme";
import { EChart } from "./EChart";

export interface ScatterPoint { x: number; y: number; label?: string }

/** Generic scatter, e.g. day strain (x) vs next-day recovery (y). */
export function ScatterPlot({ points, xName, yName, xMax, yMax, color, height = 260 }: {
  points: ScatterPoint[]; xName: string; yName: string; xMax?: number; yMax?: number; color?: string; height?: number;
}) {
  const scheme = useColorScheme();
  const option = useMemo(() => {
    const th = chartTheme();
    return {
      grid: { left: 44, right: 16, top: 12, bottom: 36 },
      tooltip: { formatter: (p: { value: number[]; data: { label?: string } }) => `${p.data.label ?? ""}<br/>${xName}: ${p.value[0]}<br/>${yName}: ${p.value[1]}` },
      xAxis: { type: "value", name: xName, nameLocation: "middle", nameGap: 22, max: xMax, nameTextStyle: { color: th.text }, axisLabel: { color: th.text }, splitLine: { lineStyle: { color: th.grid } } },
      yAxis: { type: "value", name: yName, nameTextStyle: { color: th.text, align: "left" }, max: yMax, axisLabel: { color: th.text }, splitLine: { lineStyle: { color: th.grid } } },
      series: [{ type: "scatter", symbolSize: 9, itemStyle: { color: color ? resolveColor(color) : th.strain, opacity: 0.75 }, data: points.map((p) => ({ value: [p.x, p.y], label: p.label })) }],
    };
  }, [points, xName, yName, xMax, yMax, color, scheme]);
  return <EChart option={option} height={height} ariaLabel={`${yName} versus ${xName}`} />;
}
