import { useStockAnalysisStore } from "@/stores";
import * as echarts from "echarts";
import { useEffect, useRef } from "react";

export function KLineChart() {
  const klineData = useStockAnalysisStore((s) => s.klineData);
  const chartRef = useRef<HTMLDivElement>(null);
  const instanceRef = useRef<echarts.ECharts | null>(null);

  useEffect(() => {
    if (!chartRef.current) return;
    if (!instanceRef.current) {
      instanceRef.current = echarts.init(chartRef.current);
    }
    const chart = instanceRef.current;

    if (klineData.length === 0) {
      chart.clear();
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
          name: "K线",
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
          name: "成交量",
          type: "bar",
          data: volumes,
          xAxisIndex: 1,
          yAxisIndex: 1,
        },
      ],
    });

    const handleResize = () => chart.resize();
    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  }, [klineData]);

  return <div ref={chartRef} style={{ width: "100%", height: 350 }} />;
}
