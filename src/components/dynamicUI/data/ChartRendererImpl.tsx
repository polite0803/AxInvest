// SPDX-License-Identifier: AGPL-3.0-only

import { useTranslation } from "react-i18next";
import { useIsDarkTheme } from "@/hooks/useIsDarkTheme";
import { cssVar } from "@/lib/cssVar";
import { ensureECharts } from "@/lib/echartsInstance";
import { seriesColor } from "@/theme/categoricalColors";
import { useEffect, useMemo, useRef } from "react";
import * as echarts from "echarts";

type ChartType = "line" | "bar" | "pie" | "scatter" | "area";

interface ChartRendererImplProps {
  chartType: ChartType;
  data: Record<string, unknown>[];
  xKey: string;
  yKey: string;
  seriesKey?: string;
}

function buildChartOption(
  chartType: ChartType,
  data: Record<string, unknown>[],
  xKey: string,
  yKey: string,
  seriesKey?: string,
): echarts.EChartsOption {
  const xData = data.map((d) => String(d[xKey] ?? ""));

  switch (chartType) {
    case "bar": {
      if (seriesKey) {
        return {
          tooltip: { trigger: "axis" },
          grid: { top: 16, right: 16, bottom: 32, left: 48 },
          xAxis: {
            type: "category",
            data: xData,
            axisLabel: { color: cssVar("--muted") },
          },
          yAxis: { type: "value", axisLabel: { color: cssVar("--muted") } },
          series: [
            {
              type: "bar",
              data: data.map((d, i) => ({
                value: Number(d[seriesKey] ?? 0),
                itemStyle: { color: seriesColor(i) },
              })),
            },
          ],
        };
      }
      return {
        tooltip: { trigger: "axis" },
        grid: { top: 16, right: 16, bottom: 32, left: 48 },
        xAxis: {
          type: "category",
          data: xData,
          axisLabel: { color: cssVar("--muted") },
        },
        yAxis: { type: "value", axisLabel: { color: cssVar("--muted") } },
        series: [
          {
            type: "bar",
            data: data.map((d) => Number(d[yKey] ?? 0)),
            itemStyle: { color: seriesColor(0) },
          },
        ],
      };
    }

    case "line": {
      return {
        tooltip: { trigger: "axis" },
        grid: { top: 16, right: 16, bottom: 32, left: 48 },
        xAxis: {
          type: "category",
          data: xData,
          boundaryGap: false,
          axisLabel: { color: cssVar("--muted") },
        },
        yAxis: { type: "value", axisLabel: { color: cssVar("--muted") } },
        series: [
          {
            type: "line",
            data: data.map((d) => Number(d[yKey] ?? 0)),
            smooth: true,
            showSymbol: false,
            lineStyle: { color: seriesColor(0), width: 2 },
            itemStyle: { color: seriesColor(0) },
          },
        ],
      };
    }

    case "area": {
      return {
        tooltip: { trigger: "axis" },
        grid: { top: 16, right: 16, bottom: 32, left: 48 },
        xAxis: {
          type: "category",
          data: xData,
          boundaryGap: false,
          axisLabel: { color: cssVar("--muted") },
        },
        yAxis: { type: "value", axisLabel: { color: cssVar("--muted") } },
        series: [
          {
            type: "line",
            data: data.map((d) => Number(d[yKey] ?? 0)),
            smooth: true,
            showSymbol: false,
            lineStyle: { color: seriesColor(0), width: 2 },
            itemStyle: { color: seriesColor(0) },
            areaStyle: { color: seriesColor(0), opacity: 0.3 },
          },
        ],
      };
    }

    case "pie": {
      return {
        tooltip: { trigger: "item" },
        legend: { bottom: 0, left: "center" },
        series: [
          {
            type: "pie",
            radius: "65%",
            center: ["50%", "45%"],
            data: data.map((d, i) => ({
              value: Number(d[yKey] ?? 0),
              name: String(d[xKey] ?? ""),
              itemStyle: { color: seriesColor(i) },
            })),
            label: { show: true },
          },
        ],
      };
    }

    case "scatter": {
      return {
        tooltip: { trigger: "item" },
        grid: { top: 16, right: 16, bottom: 32, left: 48 },
        xAxis: { type: "value", name: xKey, axisLabel: { color: cssVar("--muted") } },
        yAxis: { type: "value", name: yKey, axisLabel: { color: cssVar("--muted") } },
        series: [
          {
            type: "scatter",
            data: data.map((d) => [Number(d[xKey] ?? 0), Number(d[yKey] ?? 0)]),
            itemStyle: { color: seriesColor(0) },
          },
        ],
      };
    }

    default:
      return {};
  }
}

export function ChartRendererImpl({
  chartType,
  data,
  xKey,
  yKey,
  seriesKey,
}: ChartRendererImplProps) {
  const { t } = useTranslation();
  const chartRef = useRef<HTMLDivElement>(null);
  const chartInstance = useRef<echarts.ECharts | null>(null);

  const isDark = useIsDarkTheme();
  const option = useMemo(
    () => buildChartOption(chartType, data, xKey, yKey, seriesKey),
    // 系列色是构建时从 CSS 变量解析的，主题切换要重建
    [chartType, data, xKey, yKey, seriesKey, isDark],
  );

  useEffect(() => {
    if (!chartRef.current) { return; }
    chartInstance.current = ensureECharts(chartRef.current, isDark);
    chartInstance.current.setOption(option, true);
    const handleResize = () => chartInstance.current?.resize();
    window.addEventListener("resize", handleResize);
    return () => {
      window.removeEventListener("resize", handleResize);
      chartInstance.current?.dispose();
      chartInstance.current = null;
    };
  }, [option, isDark]);

  if (data.length === 0) {
    return (
      <div className="flex items-center justify-center h-64 text-gray-400">
        {t("dynamicUI.noData")}
      </div>
    );
  }

  return (
    <div
      ref={chartRef}
      style={{ width: "100%", height: 300 }}
    />
  );
}
