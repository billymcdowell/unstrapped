import { useMemo } from "react";
import { useColorScheme } from "../../lib/hooks";
import { resolveColor } from "../../lib/format";
import { chartTheme, overCard, readableOn } from "./theme";
import { EChart } from "./EChart";

export interface StackPart { label: string; value: number; color: string }
export interface StackRow { name: string; parts: StackPart[]; /** draw faded (e.g. "Typical") */ faint?: boolean }

/** Horizontal stacked bars, one per row. Used for sleep stages (with a faint typical row) and sleep need. */
export function StackedBar({ rows, height, formatValue = (v) => String(Math.round(v)), normalize }: {
  rows: StackRow[];
  height?: number;
  formatValue?: (v: number, label: string) => string;
  /** Scale each row to 100%. */
  normalize?: boolean;
}) {
  const scheme = useColorScheme();
  const h = height ?? 30 + rows.length * 34;
  const option = useMemo(() => {
    const th = chartTheme();
    const labels = [...new Set(rows.flatMap((r) => r.parts.map((p) => p.label)))];
    const totals = rows.map((r) => r.parts.reduce((s, p) => s + p.value, 0));
    return {
      grid: { left: 70, right: 12, top: 4, bottom: 4 },
      tooltip: { trigger: "item", formatter: (p: { seriesName: string; value: number; name: string }) => `${p.name} · ${p.seriesName}: ${formatValue(p.value, p.seriesName)}` },
      xAxis: { type: "value", show: false, max: normalize ? 100 : undefined },
      yAxis: { type: "category", inverse: true, data: rows.map((r) => r.name), axisLine: { show: false }, axisTick: { show: false }, axisLabel: { color: th.text } },
      series: labels.map((label) => ({
        name: label, type: "bar", stack: "t", barWidth: 16,
        itemStyle: { borderRadius: 2 },
        label: { show: true, fontSize: 11, fontWeight: 600, formatter: (p: { value: number }) => (p.value > (normalize ? 7 : 0) ? formatValue(p.value, label) : "") },
        data: rows.map((r, i) => {
          const part = r.parts.find((x) => x.label === label);
          const v = part ? (normalize ? (part.value / (totals[i] || 1)) * 100 : part.value) : 0;
          const fill = part ? (r.faint ? overCard(resolveColor(part.color), 0.4) : resolveColor(part.color)) : undefined;
          return { value: v, itemStyle: { color: fill }, label: { color: fill ? readableOn(fill) : th.textStrong } };
        }),
      })),
    };
  }, [rows, scheme, formatValue, normalize]);
  return <EChart option={option} height={h} ariaLabel="Stacked bar" />;
}
