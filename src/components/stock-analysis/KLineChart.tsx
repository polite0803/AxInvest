import { useStockAnalysisStore } from "@/stores";
import * as echarts from "echarts";
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";

export function KLineChart() {
  const { t } = useTranslation();
  const klineData = useStockAnalysisStore((s) => s.klineData);
  const chartRef = useRef<HTMLDivElement>(null);
  const instanceRef = useRef<echarts.ECharts | null>(null);

  // Effect 1: 初始化 ECharts 实例（仅在挂载/卸载时执行）
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

  // Effect 2: 更新 K 线数据（在 klineData 变化时执行）
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
      tooltip: { trigger: "axis" },
      grid: [
        { left: "8%", right: "2%", top: "2%", height: "65%" },
        { left: "8%", right: "2%", top: "75%", height: "20%" },
      ],
      xAxis: [
        { type: "category", data: dates, gridIndex: 0, axisLabel: { show: false } },
        { type: "category", data: dates, gridIndex: 1 },
      ],
      yAxis: [
        { type: "value", gridIndex: 0, scale: true },
        { type: "value", gridIndex: 1 },
      ],
      series: [
        {
          name: t("stockAnalysis.klineChart"),
          type: "candlestick",
          data: ohlc,
          xAxisIndex: 0,
          yAxisIndex: 0,
          itemStyle: {
            color: "#ef232a",
            color0: "#14b143",
            borderColor: "#ef232a",
            borderColor0: "#14b143",
          },
        },
        {
          name: t("stockAnalysis.volumeChart"),
          type: "bar",
          data: volumes,
          xAxisIndex: 1,
          yAxisIndex: 1,
        },
      ],
    });
  }, [klineData]);

  return <div ref={chartRef} style={{ width: "100%", height: 350 }} />;
}
