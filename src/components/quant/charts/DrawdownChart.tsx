// DrawdownChart — 回撤面积图

import { useIsDarkTheme } from "@/hooks/useIsDarkTheme";
import { cssVar } from "@/lib/cssVar";
import { ensureECharts } from "@/lib/echartsInstance";
import * as echarts from "echarts";
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";

import type { EquityPoint } from "@/types";

import { computeDrawdownPercent } from "./drawdown";

interface DrawdownChartProps {
  curve: EquityPoint[];
  height?: number;
}

export function DrawdownChart({ curve, height = 220 }: DrawdownChartProps) {
  const { t } = useTranslation();
  const isDark = useIsDarkTheme();
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
    // 回撤由权益曲线本地推导（Rust EquityPoint 不含 drawdown 字段）
    const data = computeDrawdownPercent(curve);

    instRef.current.setOption(
      {
        tooltip: { trigger: "axis", valueFormatter: (v: number) => `${v.toFixed(2)}%` },
        grid: { left: 50, right: 30, top: 16, bottom: 40 },
        xAxis: { type: "category", data: dates, axisLabel: { hideOverlap: true } },
        yAxis: {
          type: "value",
          name: t("quant.metrics.maxDrawdownPct"),
          max: 0,
          splitLine: { lineStyle: { color: "rgba(0,0,0,0.05)" } },
        },
        dataZoom: [{ type: "inside" }, { type: "slider", height: 14, bottom: 6 }],
        series: [
          {
            type: "line",
            showSymbol: false,
            smooth: true,
            lineStyle: { color: cssVar("--red"), width: 1.5 },
            areaStyle: { color: "rgba(207, 19, 34, 0.18)" },
            data,
          },
        ],
      },
      true,
    );
  }, [curve, t, isDark]);

  useEffect(() => {
    const onResize = () => instRef.current?.resize();
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  return <div ref={chartRef} style={{ width: "100%", height }} />;
}
