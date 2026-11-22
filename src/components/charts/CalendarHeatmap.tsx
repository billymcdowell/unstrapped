import { useMemo } from "react";
import { useColorScheme } from "../../lib/hooks";
import { chartTheme } from "./theme";
import { EChart } from "./EChart";

/** One square per day (ECharts calendar heat map). `colors` runs low to high. */
export function CalendarHeatmap({ data, from, to, min, max, colors, height = 170, unit = "" }: {
  data: { day: string; value: number }[]; from: string; to: string; min: number; max: number; colors: string[]; height?: number; unit?: string;
}) {
  const scheme = useColorScheme();
  const option = useMemo(() => {
    const th = chartTheme();
    return {
      tooltip: { formatter: (p: { value: [string, number] }) => `${p.value[0]}: ${p.value[1]}${unit}` },
      visualMap: { show: false, min, max, inRange: { color: colors } },
      calendar: {
        range: [from, to], cellSize: ["auto", 14], left: 34, right: 8, top: 22, bottom: 4,
        itemStyle: { color: th.track, borderWidth: 2, borderColor: "transparent", borderRadius: 3 },
        splitLine: { show: false }, yearLabel: { show: false },
        dayLabel: { color: th.text, firstDay: 1, nameMap: ["S", "M", "T", "W", "T", "F", "S"] },
        monthLabel: { color: th.text },
      },
      series: [{ type: "heatmap", coordinateSystem: "calendar", data: data.map((d) => [d.day, d.value]) }],
    };
  }, [data, from, to, min, max, colors, unit, scheme]);
  return <EChart option={option} height={height} ariaLabel="Calendar heat map" />;
}
