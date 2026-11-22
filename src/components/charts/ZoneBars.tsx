import { useMemo } from "react";
import { useColorScheme } from "../../lib/hooks";
import { chartTheme } from "./theme";
import { EChart } from "./EChart";

/** Five horizontal bars, minutes in HR zones 1-5. */
export function ZoneBars({ zoneMin, bounds, height = 170 }: { zoneMin: number[]; bounds?: [number, number, number, number]; height?: number }) {
  const scheme = useColorScheme();
  const option = useMemo(() => {
    const th = chartTheme();
    const name = (i: number) => (bounds ? `Zone ${i + 1}  ${i === 0 ? "<" + bounds[0] : i < 4 ? bounds[i - 1] + "–" + (bounds[i] - 1) : "≥" + bounds[3]}` : `Zone ${i + 1}`);
    return {
      grid: { left: 96, right: 56, top: 4, bottom: 4 },
      tooltip: { formatter: (p: { name: string; value: number }) => `${p.name}: ${p.value} min` },
      xAxis: { type: "value", show: false },
      yAxis: { type: "category", inverse: true, data: zoneMin.map((_, i) => name(i)), axisLine: { show: false }, axisTick: { show: false }, axisLabel: { color: th.text, fontSize: 11 } },
      series: [{
        type: "bar", barWidth: 10, showBackground: true, backgroundStyle: { color: th.track, borderRadius: 2 },
        label: { show: true, position: "right", color: th.text, formatter: (p: { value: number }) => `${Math.round(p.value)} min` },
        data: zoneMin.map((m, i) => ({ value: m, itemStyle: { color: th.zones[i], borderRadius: 2 } })),
      }],
    };
  }, [zoneMin, bounds, scheme]);
  return <EChart option={option} height={height} ariaLabel="Time in heart-rate zones" />;
}
