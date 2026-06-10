import { useStockAnalysisStore } from "@/stores";
import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";
import type { KLine } from "@/types";
import * as echarts from "echarts";
import { useCallback, useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";

/** 模块级 LRU 缓存：按 (stockCode, period, limit, asOfDate) 缓存 K 线结果
 *  asOfDate 决定"截至哪一天"——不同 as-of 下截断点不同,必须分桶,否则
 *  切换 Live ↔ Replay 会读到错误的截断后数据,违反 spec §4.1 闭世界假设。 */
interface CachedKline {
  data: KLine[];
  ts: number;
}
const klineCache = new Map<string, CachedKline>();
const KLINE_CACHE_TTL_MS = 5 * 60 * 1000; // 5 分钟
const KLINE_CACHE_MAX = 20; // 最多缓存 20 个组合

function getKlineCacheKey(stockCode: string, period: string, limit: number, asOfDate: string | null): string {
  return `${stockCode}|${period}|${limit}|${asOfDate ?? "live"}`;
}

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
  { key: "1m", limit: 22, periodType: "daily" as const },
  { key: "3m", limit: 66, periodType: "daily" as const },
  { key: "6m", limit: 120, periodType: "daily" as const },
  { key: "1y", limit: 250, periodType: "daily" as const },
  { key: "weekly", limit: 104, periodType: "weekly" as const },
  { key: "monthly", limit: 60, periodType: "monthly" as const },
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
  // 时间旅行: 顶部时间戳 react 到 store 变化
  const asOfDate = useTimeAnchorStore((s) => s.asOfDate);
  const chartRef = useRef<HTMLDivElement>(null);
  const instanceRef = useRef<echarts.ECharts | null>(null);

  /** 安全地获取 chart 实例，已 disposed 则返回 null */
  const getChart = useCallback((): echarts.ECharts | null => {
    const c = instanceRef.current;
    if (c && !c.isDisposed()) { return c; }
    instanceRef.current = null;
    return null;
  }, []);

  /** 安全地获取 chart 实例，已 disposed 则返回 null */

  const handlePeriodChange = useCallback((key: string, limit: number, periodType: string) => {
    setKlinePeriod(key);
    if (!stockCode) { return; }
    const asOfDate = useTimeAnchorStore.getState().asOfDate;
    const cacheKey = getKlineCacheKey(stockCode, periodType, limit, asOfDate);
    const cached = klineCache.get(cacheKey);
    if (cached && Date.now() - cached.ts < KLINE_CACHE_TTL_MS) {
      klineCache.delete(cacheKey);
      klineCache.set(cacheKey, cached);
      useStockAnalysisStore.setState({ klineData: cached.data });
    }
    getStockKline(stockCode, periodType, limit);
  }, [stockCode, getStockKline, setKlinePeriod]);

  // klineData 变化时把结果写入缓存（含 TTL/LRU 淘汰）
  useEffect(() => {
    if (klineData.length === 0 || !stockCode) { return; }
    const opt = PERIOD_OPTIONS.find((o) => o.key === klinePeriod);
    if (!opt) { return; }
    const asOfDate = useTimeAnchorStore.getState().asOfDate;
    const cacheKey = getKlineCacheKey(stockCode, opt.periodType, opt.limit, asOfDate);
    klineCache.set(cacheKey, { data: klineData, ts: Date.now() });
    if (klineCache.size > KLINE_CACHE_MAX) {
      const firstKey = klineCache.keys().next().value;
      if (firstKey) { klineCache.delete(firstKey); }
    }
  }, [klineData, stockCode, klinePeriod]);

  // 时间旅行：mode / asOfDate 变化时,本地缓存中其它 as-of 的桶全部失效,
  // 强制拉新数据(spec §4.1 闭世界假设)
  useEffect(() => {
    return useTimeAnchorStore.subscribe((s, prev) => {
      if (s.asOfDate !== prev.asOfDate || s.mode !== prev.mode) {
        klineCache.clear();
        if (stockCode) {
          const opt = PERIOD_OPTIONS.find((o) => o.key === klinePeriod);
          if (opt) { getStockKline(stockCode, opt.periodType, opt.limit); }
        }
      }
    });
  }, [stockCode, klinePeriod, getStockKline]);

  // 初始化 ECharts 实例 + 无条件设置观察者
  // 核心策略：ResizeObserver 和 visibilitychange 始终监听，
  // 当容器从 display:none 变为可见时自动初始化或 resize
  useEffect(() => {
    const el = chartRef.current;
    if (!el) { return; }

    const tryInit = () => {
      if (instanceRef.current && !instanceRef.current.isDisposed()) { return; }
      const w = el.clientWidth;
      const h = el.clientHeight;
      if (w === 0 || h === 0) { return; }
      // 容器已有尺寸，安全初始化
      const existing = instanceRef.current;
      if (existing && !existing.isDisposed()) { existing.dispose(); }
      instanceRef.current = echarts.init(el, undefined, { renderer: "canvas" });
    };

    const onResize = () => {
      const c = instanceRef.current;
      if (c && !c.isDisposed()) { c.resize(); }
    };

    // 立即尝试初始化（若容器已有尺寸）
    tryInit();

    // ResizeObserver：容器尺寸变化时（含从 display:none 变可见）触发
    const ro = new ResizeObserver(() => {
      onResize();
      // 若尚未初始化（之前容器为 0 尺寸），现在尝试
      tryInit();
    });
    ro.observe(el);

    // visibilitychange：浏览器标签页切换时，切回时强制 resize / 初始化
    const onVisibility = () => {
      if (document.visibilityState === "visible") {
        const c = instanceRef.current;
        if (c && !c.isDisposed()) {
          requestAnimationFrame(() => c.resize());
        } else {
          tryInit();
        }
      }
    };
    document.addEventListener("visibilitychange", onVisibility);

    // 窗口 resize
    window.addEventListener("resize", onResize);

    return () => {
      ro.disconnect();
      window.removeEventListener("resize", onResize);
      document.removeEventListener("visibilitychange", onVisibility);
      const c = instanceRef.current;
      if (c && !c.isDisposed()) { c.dispose(); }
      instanceRef.current = null;
    };
  }, []);

  // klineData 或 indicators 变化时，更新图表（不重建实例）
  useEffect(() => {
    const chart = getChart();
    if (!chart || chart.isDisposed()) { return; }
    if (klineData.length === 0) {
      chart.clear();
      return;
    }

    const dates = klineData.map((k) => k.date);
    const ohlc = klineData.map((k) => [k.open, k.close, k.low, k.high] as [number, number, number, number]);
    const volumes = klineData.map((k) => k.volume);
    const closes = klineData.map((k) => k.close);

    const ma5 = indicators.ma5 ? calcMA(closes, 5) : [];
    const ma10 = indicators.ma10 ? calcMA(closes, 10) : [];
    const ma20 = indicators.ma20 ? calcMA(closes, 20) : [];

    const candleName = t("stockAnalysis.klineChart");
    const volumeName = t("stockAnalysis.volumeChart");

    const seriesArr: echarts.SeriesOption[] = [
      {
        name: candleName,
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
      name: volumeName,
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
          const candle = arr.find((p) => p.seriesName === candleName);
          const vol = arr.find((p) => p.seriesName === volumeName);
          const idx = candle?.dataIndex ?? 0;
          const k = ohlc[idx];
          if (!k) { return ""; }
          const dateStr = dates[idx] || "";
          const raw = klineData[idx];
          const prevClose = idx > 0 ? klineData[idx - 1]?.close : null;
          const changePct = raw && prevClose ? ((raw.close - prevClose) / prevClose * 100) : null;
          const changeLabel = t("stockAnalysis.changePct");
          const changeDisplay = changePct != null
            ? `<span style="color:${changePct >= 0 ? "#ef4444" : "#22c55e"}">${changePct >= 0 ? "+" : ""}${
              changePct.toFixed(2)
            }%</span>`
            : "";

          const openLabel = t("stockAnalysis.open");
          const closeLabel = t("stockAnalysis.close");
          const highLabel = t("stockAnalysis.high");
          const lowLabel = t("stockAnalysis.low");
          const volLabel = t("stockAnalysis.volumeChart");
          const volUnit = t("stockAnalysis.volumeUnit");
          const lines = [
            `<div style="font-weight:600;margin-bottom:4px">${dateStr}</div>`,
            `${openLabel}: ${k[0].toFixed(2)} &nbsp; ${closeLabel}: ${k[1].toFixed(2)}${
              changePct != null ? ` &nbsp; ${changeLabel}: ${changeDisplay}` : ""
            }`,
            `${highLabel}: ${k[3].toFixed(2)} &nbsp; ${lowLabel}: ${k[2].toFixed(2)}`,
            vol ? `${volLabel}: ${(vol.value / 10000).toFixed(1)}${volUnit}` : "",
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
  }, [klineData, t, indicators, getChart]);

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
      {/* 第一行：时间周期切换 + MA 图例 + 时间水印 */}
      <div className="flex gap-1 mb-1 flex-wrap items-center">
        {/* 时间旅行: L4 数据水印 — 顶部时间戳横条,让用户一眼看到当前 K 线截断到哪一天 */}
        {asOfDate && (
          <span
            data-testid="kline-asof-badge"
            style={{
              display: "inline-flex",
              alignItems: "center",
              gap: 4,
              padding: "2px 8px",
              fontSize: 12,
              borderRadius: 4,
              background: "rgba(124,58,237,0.12)",
              color: "#7c3aed",
              border: "1px solid rgba(124,58,237,0.35)",
            }}
            title={t("timeTravel.replayBadge.tooltip", { date: asOfDate })}
          >
            ⏪ {t("timeTravel.pageAnchor.untilDate", { date: asOfDate })}
          </span>
        )}
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
            {t(`stockAnalysis.period.${opt.key}`)}
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
      <div ref={chartRef} style={{ width: "100%", height: chartHeight }} />
    </div>
  );
}
