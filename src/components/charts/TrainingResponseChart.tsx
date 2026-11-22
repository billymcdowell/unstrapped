import { useMemo } from "react";
import { useColorScheme } from "../../lib/hooks";
import { chartTheme } from "./theme";
import { EChart } from "./EChart";

/** Strain (bars, left axis) vs recovery (line, right axis) per month, with resting HR underneath. */
export function TrainingResponseChart({ months, strain, recovery, restingHr }: {
  months: string[]; strain: (number | null)[]; recovery: (number | null)[]; restingHr: (number | null)[];
}) {
  const scheme = useColorScheme();
  const top = useMemo(() => {
    const th = chartTheme();
    return {
      grid: { left: 40, right: 44, top: 28, bottom: 22 },
      tooltip: { trigger: "axis" },
      xAxis: { type: "category", data: months, axisTick: { show: false }, axisLine: { lineStyle: { color: th.grid } }, axisLabel: { color: th.text } },
      yAxis: [
        { type: "value", min: 0, max: 21, name: "Strain", nameTextStyle: { color: th.text, align: "left" }, axisLabel: { color: th.text }, splitLine: { lineStyle: { color: th.grid } } },
        { type: "value", min: 0, max: 100, name: "Recovery %", nameTextStyle: { color: th.text, align: "right" }, axisLabel: { color: th.text }, splitLine: { show: false } },
      ],
      series: [
        { name: "Strain", type: "bar", barMaxWidth: 28, itemStyle: { color: th.strain, borderRadius: [4, 4, 0, 0], opacity: 0.85 }, data: strain },
        { name: "Recovery", type: "line", yAxisIndex: 1, smooth: 0.2, symbolSize: 6, lineStyle: { color: th.rec.green, width: 2.5 }, itemStyle: { color: th.rec.green }, data: recovery },
      ],
    };
  }, [months, strain, recovery, scheme]);
  const bottom = useMemo(() => {
    const th = chartTheme();
    return {
      grid: { left: 40, right: 44, top: 22, bottom: 22 },
      tooltip: { trigger: "axis" },
      xAxis: { type: "category", data: months, axisTick: { show: false }, axisLine: { lineStyle: { color: th.grid } }, axisLabel: { color: th.text } },
      yAxis: { type: "value", scale: true, name: "Resting HR (bpm)", nameTextStyle: { color: th.text, align: "left" }, axisLabel: { color: th.text }, splitLine: { lineStyle: { color: th.grid } } },
      series: [{ name: "Resting HR", type: "line", smooth: 0.2, symbolSize: 6, lineStyle: { color: th.stress, width: 2 }, itemStyle: { color: th.stress }, data: restingHr }],
    };
  }, [months, restingHr, scheme]);
  return (
    <div>
      <EChart option={top} height={230} ariaLabel="Strain and recovery by month" />
      <EChart option={bottom} height={130} ariaLabel="Resting heart rate by month" />
    </div>
  );
}
