import { useMemo } from "react";
import type { SleepStage, SleepStageSegment } from "../../api/types";
import { useColorScheme } from "../../lib/hooks";
import { fmtTime } from "../../lib/date";
import { chartTheme } from "./theme";
import { EChart } from "./EChart";

const LEVEL: Record<SleepStage, number> = { awake: 3, rem: 2, light: 1, deep: 0 };
const NAME: Record<SleepStage, string> = { awake: "Awake", rem: "REM", light: "Light", deep: "Deep" };

/** Step chart of the night: Awake, REM, Light, Deep top to bottom; disturbance ticks on the Awake row. */
export function Hypnogram({ segments, disturbancesMs = [], height = 200 }: { segments: SleepStageSegment[]; disturbancesMs?: number[]; height?: number }) {
  const scheme = useColorScheme();
  const option = useMemo(() => {
    const th = chartTheme();
    const start = segments[0]?.startMs, end = segments[segments.length - 1]?.endMs;
    return {
      grid: { left: 52, right: 12, top: 10, bottom: 26 },
      tooltip: {
        formatter: (p: { seriesType: string; value: number[] }) =>
          p.seriesType === "custom"
            ? `${NAME[(Object.keys(LEVEL) as SleepStage[]).find((k) => LEVEL[k] === p.value[2])!]}<br/>${fmtTime(p.value[0])}–${fmtTime(p.value[1])}`
            : `Disturbance ${fmtTime(p.value[0])}`,
      },
      xAxis: { type: "time", min: start, max: end, axisLabel: { color: th.text, formatter: (v: number) => fmtTime(v) }, axisLine: { lineStyle: { color: th.grid } }, splitLine: { show: false } },
      yAxis: {
        type: "value", min: -1, max: 4, interval: 1,
        axisLabel: { color: th.text, formatter: (v: number) => ["Deep", "Light", "REM", "Awake"][v] ?? "" },
        splitLine: { lineStyle: { color: th.grid } },
      },
      series: [
        {
          type: "custom",
          data: segments.map((s) => [s.startMs, s.endMs, LEVEL[s.stage], s.stage]),
          renderItem: (_p: unknown, api: any) => {
            const a = api.coord([api.value(0), api.value(2)]);
            const b = api.coord([api.value(1), api.value(2)]);
            const stage = api.value(3) as SleepStage;
            return { type: "rect", shape: { x: a[0], y: a[1] - 7, width: Math.max(1, b[0] - a[0]), height: 14, r: 2 }, style: { fill: th.stages[stage] } };
          },
        },
        { type: "scatter", symbol: "triangle", symbolSize: 8, itemStyle: { color: th.stages.awake }, data: disturbancesMs.map((t) => [t, 3.65]), tooltip: { show: true } },
      ],
    };
  }, [segments, disturbancesMs, scheme]);
  return <EChart option={option} height={height} ariaLabel="Sleep stages over the night" />;
}
