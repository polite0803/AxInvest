import { useStockAnalysisStore } from "@/stores";
import * as echarts from "echarts";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

/** 计算移动平均线 */
function calcMA(data: number[], window: number): (number | null)[] {
  return data.map((_, i) => {
    if (i < window - 1) { return null; }
    let sum = 0;
    for (let j = i - window + 1; j <= i; j++) { sum += data[j]; }
    return parseFloat((sum / window).toFixed(2));
  });
}

const PERIOD_OPTIONS = [
  { key: "1m", label: "1月", limit: 22, periodType: "daily" as const },
  { key: "3m", label: "3月", limit: 66, periodType: "daily" as const },
  { key: "6m", label: "6月", limit: 120, periodType: "daily" as const },
  { key: "1y", label: "1年", limit: 250, periodType: "daily" as const },
  { key: "weekly", label: "周", limit: 104, periodType: "weekly" as const },
  { key: "monthly", label: "月", limit: 60, periodType: "monthly" as const },
] as const;

/** ECharts 画布用色 — 与 index.css --sa-* 变量保持同步（OKLch 值） */
const SA_RED = "oklch(60% 0.20 30)";
const SA_GREEN = "oklch(62% 0.18 150)";
const MA_ORANGE = "oklch(68% 0.16 50)";
const MA_BLUE = "oklch(55% 0.14 250)";
const MA_PURPLE = "oklch(55% 0.14 310)";

export function KLineChart() {
  const { t } = useTranslation();
  const klineData = useStockAnalysisStore((s) => s.klineData);
  const stockCode = useStockAnalysisStore((s) => s.stockCode);
  const klinePeriod = useStockAnalysisStore((s) => s.klinePeriod);
  const setKlinePeriod = useStockAnalysisStore((s) => s.setKlinePeriod);
  const indicators = useStockAnalysisStore((s) => s.klineIndicators);
  const toggleIndicator = useStockAnalysisStore((s) => s.toggleIndicator);
  const getStockKline = useStockAnalysisStore((s) => s.getStockKline);
  const chartRef = useRef<HTMLDivElement>(null);
  const instanceRef = useRef<echarts.ECharts | null>(null);

  const handlePeriodChange = useCallback((key: string, limit: number, periodType: string) => {
    setKlinePeriod(key);
    if (stockCode) {
      getStockKline(stockCode, periodType, limit);
    }
  }, [stockCode, getStockKline, setKlinePeriod]);

  useEffect(() => {
    if (!chartRef.current) { return; }
    instanceRef.current = echarts.init(chartRef.current, undefined, { renderer: "canvas" });
    setChartReady(true);
    const chart = instanceRef.current;
    const handleResize = () => chart.resize();
    window.addEventListener("resize", handleResize);
    return () => {
      window.removeEventListener("resize", handleResize);
      chart.dispose();
      instanceRef.current = null;
    };
  }, []);

  useEffect(() => {
    const chart = instanceRef.current;
    if (!chart || klineData.length === 0) {
      chart?.clear();
      return;
    }

    const dates = klineData.map((k) => k.date);
    const ohlc = klineData.map((k) => [k.open, k.close, k.low, k.high]);
    const volumes = klineData.map((k) => k.volume);
    const closes = klineData.map((k) => k.close);

    const ma5 = indicators.ma5 ? calcMA(closes, 5) : [];
    const ma10 = indicators.ma10 ? calcMA(closes, 10) : [];
    const ma20 = indicators.ma20 ? calcMA(closes, 20) : [];

    const seriesArr: echarts.SeriesOption[] = [
      {
        name: "K线",
        type: "candlestick",
        data: ohlc,
        xAxisIndex: 0,
        yAxisIndex: 0,
        itemStyle: { color: SA_RED, color0: SA_GREEN, borderColor: SA_RED, borderColor0: SA_GREEN },
      },
    ];

    if (indicators.ma5) {
      seriesArr.push({
        name: "MA5",
        type: "line",
        data: ma5,
        xAxisIndex: 0,
        yAxisIndex: 0,
        smooth: true,
        showSymbol: false,
        lineStyle: { width: 1.5, color: MA_ORANGE },
        z: 1,
      });
    }
    if (indicators.ma10) {
      seriesArr.push({
        name: "MA10",
        type: "line",
        data: ma10,
        xAxisIndex: 0,
        yAxisIndex: 0,
        smooth: true,
        showSymbol: false,
        lineStyle: { width: 1.5, color: MA_BLUE },
        z: 1,
      });
    }
    if (indicators.ma20) {
      seriesArr.push({
        name: "MA20",
        type: "line",
        data: ma20,
        xAxisIndex: 0,
        yAxisIndex: 0,
        smooth: true,
        showSymbol: false,
        lineStyle: { width: 1.5, color: MA_PURPLE },
        z: 1,
      });
    }

    seriesArr.push({
      name: "成交量",
      type: "bar",
      data: volumes,
      xAxisIndex: 1,
      yAxisIndex: 1,
      itemStyle: {
        color: (params: { dataIndex: number }) => {
          const k = ohlc[params.dataIndex];
          return k && k[1] >= k[0] ? SA_RED : SA_GREEN;
        },
      },
    });

    chart.setOption({
      animation: true,
      animationDuration: 300,
      tooltip: {
        trigger: "axis",
        axisPointer: { type: "cross" },
        backgroundColor: "var(--surface)",
        borderColor: "var(--border)",
        borderWidth: 1,
        textStyle: { fontSize: 12, color: "var(--fg)" },
        formatter: (params: unknown) => {
          const arr = params as Array<{ seriesName: string; value: number; dataIndex: number }>;
          if (!arr || arr.length === 0) { return ""; }
          const candle = arr.find((p) => p.seriesName === "K线");
          const vol = arr.find((p) => p.seriesName === "成交量");
          const idx = candle?.dataIndex ?? 0;
          const k = ohlc[idx];
          if (!k) { return ""; }
          const dateStr = dates[idx] || "";
          const lines = [
            `<div style="font-weight:600;margin-bottom:4px">${dateStr}</div>`,
            `开: ${k[0].toFixed(2)} &nbsp; 收: ${k[1].toFixed(2)}`,
            `高: ${k[2].toFixed(2)} &nbsp; 低: ${k[3].toFixed(2)}`,
            vol ? `成交量: ${(vol.value / 10000).toFixed(1)}万手` : "",
          ];
          if (indicators.ma5 && ma5[idx] != null) {
            lines.push(`<span style="color:${MA_ORANGE}">MA5: ${ma5[idx]}</span>`);
          }
          if (indicators.ma10 && ma10[idx] != null) {
            lines.push(`<span style="color:${MA_BLUE}">MA10: ${ma10[idx]}</span>`);
          }
          if (indicators.ma20 && ma20[idx] != null) {
            lines.push(`<span style="color:${MA_PURPLE}">MA20: ${ma20[idx]}</span>`);
          }
          return lines.filter(Boolean).join("<br>");
        },
      },
      axisPointer: { link: [{ xAxisIndex: "all" }] },
      toolbox: {
        right: 10,
        feature: {
          dataZoom: {
            yAxisIndex: false,
            title: { zoom: t("stockAnalysis.chart.zoom"), back: t("stockAnalysis.chart.restore") },
          },
          restore: { title: t("stockAnalysis.chart.restore") },
        },
      },
      dataZoom: [
        { type: "inside", xAxisIndex: [0, 1], start: 60, end: 100 },
        { type: "slider", xAxisIndex: [0, 1], bottom: 0, height: 20, borderColor: "var(--border)" },
      ],
      grid: [
        { left: "3%", right: "3%", top: 30, height: "58%" },
        { left: "3%", right: "3%", top: "72%", height: "18%" },
      ],
      xAxis: [
        { type: "category", data: dates, gridIndex: 0, axisLabel: { show: false }, boundaryGap: true },
        { type: "category", data: dates, gridIndex: 1, axisLabel: { rotate: 45, fontSize: 10 }, boundaryGap: true },
      ],
      yAxis: [
        {
          type: "value",
          gridIndex: 0,
          scale: true,
          splitArea: { show: true, areaStyle: { color: ["rgba(0,0,0,0.02)", "rgba(0,0,0,0.04)"] } },
          axisLabel: { fontSize: 10 },
        },
        { type: "value", gridIndex: 1, axisLabel: { show: false }, splitLine: { show: false } },
      ],
      series: seriesArr,
    });
  }, [klineData, t, indicators]);

  const [chartReady, setChartReady] = useState(false);

  if (klineData.length === 0) {
    return (
      <div
        className="flex items-center justify-center"
        style={{ minHeight: 200, color: "var(--muted)", fontSize: 13 }}
      >
        {t("stockAnalysis.noChartData")}
      </div>
    );
  }

  const chartHeight = Math.max(300, Math.min(520, window.innerHeight * 0.38));

  return (
    <div>
      {/* 第一行：时间周期切换 + MA 图例 */}
      <div className="flex gap-1 mb-1 flex-wrap items-center">
        {PERIOD_OPTIONS.map((opt) => (
          <button
            key={opt.key}
            onClick={() => handlePeriodChange(opt.key, opt.limit, opt.periodType)}
            style={{
              padding: "2px 8px",
              fontSize: 11,
              borderRadius: 4,
              border: `1px solid ${klinePeriod === opt.key ? "var(--accent)" : "var(--border)"}`,
              background: klinePeriod === opt.key ? "var(--accent-dim)" : "transparent",
              color: klinePeriod === opt.key ? "var(--accent)" : "var(--muted)",
              cursor: "pointer",
              whiteSpace: "nowrap",
            }}
          >
            {opt.label}
          </button>
        ))}
      </div>
      {/* 第二行：MA 图例 + 开关 */}
      <div
        className="flex gap-3 mb-1 flex-wrap items-center text-xs"
        style={{ color: "var(--muted)" }}
      >
        <label className="flex items-center gap-1 cursor-pointer select-none" onClick={() => toggleIndicator("ma5")}>
          <input
            type="checkbox"
            checked={indicators.ma5}
            onChange={() => toggleIndicator("ma5")}
            style={{ width: 12, height: 12 }}
          />
          <span style={{ color: MA_ORANGE }}>━</span> MA5
        </label>
        <label className="flex items-center gap-1 cursor-pointer select-none" onClick={() => toggleIndicator("ma10")}>
          <input
            type="checkbox"
            checked={indicators.ma10}
            onChange={() => toggleIndicator("ma10")}
            style={{ width: 12, height: 12 }}
          />
          <span style={{ color: MA_BLUE }}>━</span> MA10
        </label>
        <label className="flex items-center gap-1 cursor-pointer select-none" onClick={() => toggleIndicator("ma20")}>
          <input
            type="checkbox"
            checked={indicators.ma20}
            onChange={() => toggleIndicator("ma20")}
            style={{ width: 12, height: 12 }}
          />
          <span style={{ color: MA_PURPLE }}>━</span> MA20
        </label>
      </div>
      {!chartReady && <div className="ax-skeleton" style={{ width: "100%", height: chartHeight, borderRadius: 6 }} />}
      <div ref={chartRef} style={{ width: "100%", height: chartHeight, display: chartReady ? "block" : "none" }} />
    </div>
  );
}
