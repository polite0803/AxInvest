// EquityCurveChart — 收益曲线 + 基准对比
// 用 ECharts line + area

import { useIsDarkTheme } from "@/hooks/useIsDarkTheme";
import { cssVar } from "@/lib/cssVar";
import { ensureECharts } from "@/lib/echartsInstance";
import { hueColor } from "@/theme/categoricalColors";
import * as echarts from "echarts";
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";

import type { EquityPoint } from "@/types";

interface EquityCurveChartProps {
  curve: EquityPoint[];
  benchmark?: number[]; // 同一时间序列的归一化基准收益（可选）
  benchmarkName?: string;
  height?: number;
}

export function EquityCurveChart({
  curve,
  benchmark,
  benchmarkName,
  height = 360,
}: EquityCurveChartProps) {
  const { t } = useTranslation();
  const isDark = useIsDarkTheme();
  const benchLabel = benchmarkName ?? t("benchmark.name");
  const chartRef = useRef<HTMLDivElement>(null);
  const instRef = useRef<echarts.ECharts | null>(null);

  useEffect(() => {
    if (!chartRef.current) { return; }
    // 主题档变化必须重建实例：ECharts 的坐标轴文字 / 图例 / tooltip 底色取自 init 时的主题，
    // setOption 只覆盖我们显式写的那些颜色，暗色下不给 "dark" 就是深字压深底。
    instRef.current = ensureECharts(chartRef.current, isDark);
    return () => {
      instRef.current?.dispose();
      instRef.current = null;
    };
  }, [isDark]);

  useEffect(() => {
    if (!instRef.current || curve.length === 0) { return; }

    const dates = curve.map((p) => p.date);
    const equities = curve.map((p) => parseFloat(p.equity.toFixed(2)));

    const series: echarts.SeriesOption[] = [
      {
        name: t("quant.metrics.equity"),
        type: "line",
        smooth: true,
        showSymbol: false,
        sampling: "lttb",
        lineStyle: { width: 2, color: hueColor("blue", isDark) },
        areaStyle: { color: "rgba(22, 119, 255, 0.12)" },
        data: equities,
      },
    ];
    if (benchmark && benchmark.length === curve.length) {
      const init = curve[0].equity;
      const norm = benchmark.map((r) => init * (1 + r));
      series.push({
        name: benchLabel,
        type: "line",
        smooth: true,
        showSymbol: false,
        sampling: "lttb",
        lineStyle: { width: 1.5, color: cssVar("--muted"), type: "dashed" },
        data: norm.map((v) => parseFloat(v.toFixed(2))),
      });
    }

    instRef.current.setOption(
      {
        tooltip: { trigger: "axis" },
        legend: { top: 0, data: benchmark ? [t("quant.metrics.equity"), benchLabel] : [t("quant.metrics.equity")] },
        grid: { left: 56, right: 56, top: 30, bottom: 50 },
        xAxis: { type: "category", data: dates, axisLabel: { hideOverlap: true } },
        yAxis: [
          {
            type: "value",
            name: t("quant.metrics.equity"),
            scale: true,
            splitLine: { lineStyle: { color: "rgba(0,0,0,0.05)" } },
          },
        ],
        dataZoom: [
          { type: "inside" },
          { type: "slider", height: 18, bottom: 8 },
        ],
        series,
      },
      true,
    );
  }, [curve, benchmark, benchLabel, t, isDark]);

  useEffect(() => {
    const onResize = () => instRef.current?.resize();
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  return <div ref={chartRef} style={{ width: "100%", height }} />;
}
