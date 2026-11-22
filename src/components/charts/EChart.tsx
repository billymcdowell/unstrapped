import { useEffect, useRef } from "react";
import * as echarts from "echarts/core";
import { BarChart, CustomChart, HeatmapChart, LineChart, ScatterChart } from "echarts/charts";
import {
  CalendarComponent, GridComponent, MarkAreaComponent, MarkLineComponent, TooltipComponent, VisualMapComponent,
} from "echarts/components";
import { CanvasRenderer } from "echarts/renderers";
import type { EChartsCoreOption } from "echarts/core";

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
    chart.current?.setOption({ backgroundColor: "transparent", animationDuration: 400, textStyle: { fontFamily: "inherit" }, ...option }, true);
  }, [option]);

  return <div ref={el} className="chart" style={{ height }} role="img" aria-label={ariaLabel} />;
}
