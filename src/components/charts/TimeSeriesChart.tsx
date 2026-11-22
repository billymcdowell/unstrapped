import { useEffect, useRef } from "react";
import uPlot from "uplot";
import "uplot/dist/uPlot.min.css";
import type { Span, TimeSeries } from "../../api/types";
import { useColorScheme } from "../../lib/hooks";
import { resolveColor } from "../../lib/format";
import { chartTheme } from "./theme";

export interface TimeSeriesChartProps {
  series: TimeSeries;
  height?: number;
  /** Line colour (CSS colour or var()). Ignored when `zones` is set. */
  color?: string;
  /** Colour the line by y value: from <= y < to. Used for HR zones. */
  zones?: { from: number; to: number; color: string }[];
  /** Horizontal background bands, e.g. stress low/medium/high. */
  bands?: { from: number; to: number; color: string }[];
  /** Vertical shaded spans: sleep, activities, gaps. */
  spans?: Span[];
  yRange?: [number, number];
  /** Fixed x window in ms; defaults to the data extent. */
  xRange?: [number, number];
  fill?: boolean;
  ariaLabel?: string;
}

const SPAN_COLOR: Record<Span["kind"], string> = {
  sleep: "--span-sleep", nap: "--span-sleep", activity: "--span-activity", gap: "--span-gap",
};

/** uPlot wrapper for long time series (HR, stress). Gaps (null) break the line. Re-reads theme tokens on light/dark change. */
export function TimeSeriesChart({ series, height = 220, color = "var(--accent)", zones, bands, spans, yRange, xRange, fill, ariaLabel }: TimeSeriesChartProps) {
  const host = useRef<HTMLDivElement>(null);
  const scheme = useColorScheme();

  useEffect(() => {
    const node = host.current;
    if (!node) return;
    const th = chartTheme();
    const lineColor = resolveColor(color);
    const bandColors = bands?.map((b) => ({ ...b, color: resolveColor(b.color) }));
    const zoneColors = zones?.map((z) => ({ ...z, color: resolveColor(z.color) }));
    const spanFill = (k: Span["kind"]) => resolveColor(`var(${SPAN_COLOR[k]})`);

    const stroke: uPlot.Series["stroke"] = zoneColors
      ? (u) => {
          const { top, height: h } = u.bbox;
          const g = u.ctx.createLinearGradient(0, top, 0, top + h);
          const off = (v: number) => Math.min(1, Math.max(0, (u.valToPos(v, "y", true) - top) / h));
          [...zoneColors].sort((a, b) => b.from - a.from).forEach((z) => {
            g.addColorStop(off(Math.min(z.to, 1e6)), z.color);
            g.addColorStop(off(z.from), z.color);
          });
          return g;
        }
      : lineColor;

    const data: uPlot.AlignedData = [series.ts.map((t) => t / 1000), series.v as (number | null)[] as number[]];
    const xs = xRange ?? (series.ts.length ? [series.ts[0], series.ts[series.ts.length - 1]] : [0, 1]);

    const opts: uPlot.Options = {
      width: node.clientWidth || 600,
      height,
      padding: [8, 8, 0, 0],
      legend: { show: false },
      cursor: { drag: { x: false, y: false }, points: { size: 7 } },
      scales: {
        x: { time: true, range: () => [xs[0] / 1000, xs[1] / 1000] },
        y: { range: yRange ? () => yRange : (_u, min, max) => [Math.max(0, Math.floor((min ?? 0) - 5)), Math.ceil((max ?? 100) + 5)] },
      },
      axes: [
        { stroke: th.text, grid: { stroke: th.grid, width: 1 }, ticks: { show: false }, font: `11px ${th.font}` },
        { stroke: th.text, grid: { stroke: th.grid, width: 1 }, ticks: { show: false }, size: 38, font: `11px ${th.font}` },
      ],
      series: [
        {},
        { stroke, width: 1.75, spanGaps: false, points: { show: false }, fill: fill ? `${lineColor}22` : undefined },
      ],
      hooks: {
        drawClear: [
          (u) => {
            const { ctx, bbox } = u;
            ctx.save();
            bandColors?.forEach((b) => {
              const y0 = u.valToPos(b.to, "y", true), y1 = u.valToPos(b.from, "y", true);
              ctx.fillStyle = b.color;
              ctx.fillRect(bbox.left, y0, bbox.width, y1 - y0);
            });
            spans?.forEach((s) => {
              const x0 = Math.max(bbox.left, u.valToPos(s.startMs / 1000, "x", true));
              const x1 = Math.min(bbox.left + bbox.width, u.valToPos(s.endMs / 1000, "x", true));
              if (x1 <= x0) return;
              ctx.fillStyle = spanFill(s.kind);
              ctx.fillRect(x0, bbox.top, x1 - x0, bbox.height);
            });
            ctx.restore();
          },
        ],
      },
    };

    const plot = new uPlot(opts, data, node);
    const ro = new ResizeObserver(() => plot.setSize({ width: node.clientWidth, height }));
    ro.observe(node);
    return () => { ro.disconnect(); plot.destroy(); };
  }, [series, height, color, zones, bands, spans, yRange, xRange, fill, scheme]);

  return <div ref={host} className="chart chart-uplot" style={{ height }} role="img" aria-label={ariaLabel} />;
}
