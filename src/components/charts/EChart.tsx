import { useEffect, useRef } from "react";
import * as echarts from "echarts/core";
import { BarChart, CustomChart, HeatmapChart, LineChart, ScatterChart } from "echarts/charts";
import {
  CalendarComponent, GridComponent, MarkAreaComponent, MarkLineComponent, TooltipComponent, VisualMapComponent,
} from "echarts/components";
import { CanvasRenderer } from "echarts/renderers";
import type { EChartsCoreOption } from "echarts/core";
import { cssVar } from "../../lib/format";

echarts.use([
  BarChart, CustomChart, HeatmapChart, LineChart, ScatterChart,
  CalendarComponent, GridComponent, MarkAreaComponent, MarkLineComponent, TooltipComponent, VisualMapComponent,
  CanvasRenderer,
]);

export interface EChartProps {
  option: EChartsCoreOption;
  height: number;
  ariaLabel?: string;
  onClick?: (params: unknown) => void;
}

/** Base ECharts host: resizes with its container, transparent background so it inherits the card. */
export function EChart({ option, height, ariaLabel, onClick }: EChartProps) {
  const el = useRef<HTMLDivElement>(null);
  const chart = useRef<echarts.ECharts | null>(null);
  const clickRef = useRef(onClick);
  clickRef.current = onClick;

  useEffect(() => {
    if (!el.current) return;
    const c = echarts.init(el.current, undefined, { renderer: "canvas" });
    chart.current = c;
    c.on("click", (p) => clickRef.current?.(p));
    const ro = new ResizeObserver(() => c.resize());
    ro.observe(el.current);
    return () => { ro.disconnect(); c.dispose(); chart.current = null; };
  }, []);

  useEffect(() => {
    const o = option as Record<string, unknown>;
    const tooltip = o.tooltip
      ? {
          backgroundColor: cssVar("--card"), borderWidth: 0, padding: [8, 12],
          textStyle: { color: cssVar("--text"), fontSize: 12, fontFamily: cssVar("--font") },
          extraCssText: "border-radius:6px;box-shadow:0 10px 30px -10px rgba(0,0,0,.25);",
          ...(o.tooltip as object),
        }
      : undefined;
    chart.current?.setOption({ backgroundColor: "transparent", animationDuration: 500, animationEasing: "cubicOut", textStyle: { fontFamily: cssVar("--font"), fontSize: 11 }, ...option, tooltip }, true);
  }, [option]);

  return <div ref={el} className="chart" style={{ height }} role="img" aria-label={ariaLabel} />;
}
