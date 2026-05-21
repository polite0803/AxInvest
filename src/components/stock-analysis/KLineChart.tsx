import { useStockAnalysisStore } from "@/stores";
import * as echarts from "echarts";
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";

export function KLineChart() {
  const { t } = useTranslation();
  const klineData = useStockAnalysisStore((s) => s.klineData);
  const chartRef = useRef<HTMLDivElement>(null);
  const instanceRef = useRef<echarts.ECharts | null>(null);

  useEffect(() => {
    if (!chartRef.current) { return; }
    instanceRef.current = echarts.init(chartRef.current);
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

    chart.setOption({
      animation: false,
      tooltip: {
        trigger: "axis",
        axisPointer: { type: "cross" },
      },
      axisPointer: {
        link: [{ xAxisIndex: "all" }],
      },
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
        { type: "slider", xAxisIndex: [0, 1], bottom: 0, height: 20 },
      ],
      grid: [
        { left: "8%", right: "8%", top: 30, height: "60%" },
        { left: "8%", right: "8%", top: "72%", height: "18%" },
      ],
      xAxis: [
        { type: "category", data: dates, gridIndex: 0, axisLabel: { show: false }, boundaryGap: true },
        { type: "category", data: dates, gridIndex: 1, axisLabel: { rotate: 45 }, boundaryGap: true },
      ],
      yAxis: [
        { type: "value", gridIndex: 0, scale: true, splitArea: { show: true } },
        { type: "value", gridIndex: 1, axisLabel: { show: false }, splitLine: { show: false } },
      ],
      series: [
        {
          name: t("stockAnalysis.klineChart"),
          type: "candlestick",
          data: ohlc,
          xAxisIndex: 0,
          yAxisIndex: 0,
          itemStyle: { color: "#ef232a", color0: "#14b143", borderColor: "#ef232a", borderColor0: "#14b143" },
        },
        {
          name: t("stockAnalysis.volumeChart"),
          type: "bar",
          data: volumes,
          xAxisIndex: 1,
          yAxisIndex: 1,
          itemStyle: {
            color: (params: { dataIndex: number }) => {
              const k = ohlc[params.dataIndex];
              return k && k[1] >= k[0] ? "#ef232a" : "#14b143";
            },
          },
        },
      ],
    });
  }, [klineData]);

  return <div ref={chartRef} style={{ width: "100%", height: 420 }} />;
}
