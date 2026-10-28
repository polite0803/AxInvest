// WalkForwardFoldBarChart — per-fold 训练 vs 测试 Sharpe 柱状图
//
// 目的：直观看到每个 fold 的 IS/OOS 性能退化，识别过拟合 fold
// 视觉：
// - X 轴：fold 索引（0..N-1）
// - 柱状：train Sharpe（蓝） vs test Sharpe（绿/红：过拟合标红）
// - 折线：degradationRatio（test/train），右侧 Y 轴
// - 0.3 阈值线：低于则视为过拟合
// - tooltip：fold 区间 + 关键指标

import { useIsDarkTheme } from "@/hooks/useIsDarkTheme";
import { cssVar } from "@/lib/cssVar";
import { ensureECharts } from "@/lib/echartsInstance";
import { hueColor } from "@/theme/categoricalColors";
import * as echarts from "echarts";
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";

import type { WalkForwardWindowResult } from "@/types/quant";

interface WalkForwardFoldBarChartProps {
  windows: WalkForwardWindowResult[];
  height?: number;
}

export function WalkForwardFoldBarChart({ windows, height = 280 }: WalkForwardFoldBarChartProps) {
  const { t } = useTranslation();
  const isDark = useIsDarkTheme();
  const chartRef = useRef<HTMLDivElement>(null);
  const instRef = useRef<echarts.ECharts | null>(null);

  // 主题档变化必须重建实例：ECharts 的坐标轴文字 / 图例 / tooltip 底色取自 init 时的
  // 主题，setOption 只覆盖我们显式写的那些颜色，暗色下不给 "dark" 就是深字压深底。
  useEffect(() => {
    if (!chartRef.current) { return; }
    instRef.current = ensureECharts(chartRef.current, isDark);
    return () => {
      instRef.current?.dispose();
      instRef.current = null;
    };
  }, [isDark]);

  useEffect(() => {
    if (!instRef.current || windows.length === 0) { return; }

    const foldLabels = windows.map((w) => `F${w.fold.foldIndex}`);
    const trainSharpe = windows.map((w) => parseFloat(w.trainMetrics.sharpe.toFixed(3)));
    const testSharpe = windows.map((w) => parseFloat(w.testMetrics.sharpe.toFixed(3)));
    // test_bar 颜色：过拟合 fold 红色，否则绿色
    // 过拟合标红 / 正常标绿走语义 token；训练柱与退化率折线走类目色板（两柱并排必须可辨）
    const testColors = windows.map((w) => (w.overfitFlag ? cssVar("--red") : cssVar("--green")));
    const degradation = windows.map((w) => parseFloat(w.degradationRatio.toFixed(3)));

    instRef.current.setOption(
      {
        tooltip: {
          trigger: "axis",
          axisPointer: { type: "shadow" },
          formatter: (params: unknown) => {
            const arr = params as Array<{ dataIndex: number }>;
            const w = windows[arr[0].dataIndex];
            const f = w.fold;
            return [
              `<b>Fold ${f.foldIndex}</b> ${
                w.overfitFlag ? `<span style='color:var(--red)'>(${t("quant.metrics.overfit")})</span>` : ""
              }`,
              `IS: ${f.trainStart} → ${f.trainEnd} (${t("quant.backtest.barsCount", { count: f.trainBarsCount })})`,
              `OOS: ${f.testStart} → ${f.testEnd} (${t("quant.backtest.barsCount", { count: f.testBarsCount })})`,
              `Train Sharpe: ${w.trainMetrics.sharpe.toFixed(3)}`,
              `Test  Sharpe: ${w.testMetrics.sharpe.toFixed(3)}`,
              `Degradation: ${w.degradationRatio.toFixed(3)} ${w.overfitFlag ? "⚠️" : ""}`,
            ].join("<br/>");
          },
        },
        legend: {
          top: 0,
          data: ["Train Sharpe", "Test Sharpe", "Degradation Ratio"],
        },
        grid: { left: 60, right: 60, top: 36, bottom: 40 },
        xAxis: {
          type: "category",
          data: foldLabels,
          name: "Fold",
          axisLabel: { interval: 0 },
        },
        yAxis: [
          {
            type: "value",
            name: "Sharpe",
            splitLine: { lineStyle: { color: cssVar("--border") } },
          },
          {
            type: "value",
            name: "Degradation",
            position: "right",
            min: 0,
            max: 1.2,
            splitLine: { show: false },
          },
        ],
        dataZoom: windows.length > 12 ? [{ type: "inside" }, { type: "slider", height: 14, bottom: 4 }] : [],
        series: [
          {
            name: "Train Sharpe",
            type: "bar",
            data: trainSharpe,
            itemStyle: { color: hueColor("blue", isDark) },
            barGap: 0,
          },
          {
            name: "Test Sharpe",
            type: "bar",
            data: testSharpe.map((v, i) => ({ value: v, itemStyle: { color: testColors[i] } })),
          },
          {
            name: "Degradation Ratio",
            type: "line",
            yAxisIndex: 1,
            smooth: false,
            showSymbol: true,
            symbolSize: 6,
            lineStyle: { width: 1.5, color: hueColor("orange", isDark), type: "dashed" },
            itemStyle: { color: hueColor("orange", isDark) },
            data: degradation,
            markLine: {
              silent: true,
              symbol: "none",
              lineStyle: { color: cssVar("--red"), type: "dotted", width: 1 },
              data: [{
                yAxis: 0.3,
                label: { formatter: `${t("quant.metrics.overfit")} ${t("quant.chart.threshold", { value: "0.3" })}` },
              }],
            },
          },
        ],
      },
      true,
    );
  }, [windows, t, isDark]);

  useEffect(() => {
    const onResize = () => instRef.current?.resize();
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  return <div ref={chartRef} style={{ width: "100%", height }} />;
}
