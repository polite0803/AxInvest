import { invoke } from "@/lib/invoke";
import type { StockQuote } from "@/types";
import { SwapOutlined } from "@ant-design/icons";
import { Button, Card, Input } from "antd";
import * as echarts from "echarts";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

function CompareRow({ label, v1, v2, color1, color2 }: {
  label: string;
  v1: string | number;
  v2: string | number;
  color1?: string;
  color2?: string;
}) {
  return (
    <div
      className="flex items-center text-xs"
      style={{ borderBottom: "1px solid var(--border-color, #f0f0f0)", padding: "4px 0" }}
    >
      <span className="shrink-0" style={{ width: 48, color: "var(--color-text-secondary)" }}>{label}</span>
      <span className="flex-1 text-right font-mono" style={{ color: color1 }}>{v1}</span>
      <span className="flex-1 text-right font-mono" style={{ color: color2 }}>{v2}</span>
    </div>
  );
}

export function CompareView() {
  const { t } = useTranslation();
  const [code1, setCode1] = useState("");
  const [code2, setCode2] = useState("");
  const [quote1, setQuote1] = useState<StockQuote | null>(null);
  const [quote2, setQuote2] = useState<StockQuote | null>(null);
  const [loading, setLoading] = useState(false);
  const radarRef = useRef<HTMLDivElement>(null);
  const radarInstance = useRef<echarts.ECharts | null>(null);

  const compare = async () => {
    if (!code1 || !code2) { return; }
    setLoading(true);
    try {
      const [q1, q2] = await Promise.all([
        invoke<StockQuote>("get_stock_quote", { stockCode: code1 }),
        invoke<StockQuote>("get_stock_quote", { stockCode: code2 }),
      ]);
      setQuote1(q1);
      setQuote2(q2);
    } catch {
      // 静默处理
    } finally {
      setLoading(false);
    }
  };

  // 雷达图对比
  useEffect(() => {
    if (!radarRef.current) { return; }
    if (!radarInstance.current) {
      radarInstance.current = echarts.init(radarRef.current, undefined, { renderer: "canvas" });
    }
    const chart = radarInstance.current;

    if (!quote1 || !quote2) {
      chart.clear();
      return;
    }

    const labels = [
      t("stockAnalysis.price"),
      "PE",
      "PB",
      t("stockAnalysis.volume"),
      t("stockAnalysis.turnoverRate"),
    ];
    const raw1 = [
      quote1.price,
      quote1.pe ?? 0,
      quote1.pb ?? 0,
      quote1.volume / 10000,
      quote1.turnoverRate ?? 0,
    ];
    const raw2 = [
      quote2.price,
      quote2.pe ?? 0,
      quote2.pb ?? 0,
      quote2.volume / 10000,
      quote2.turnoverRate ?? 0,
    ];
    const maxes = raw1.map((v, i) => Math.max(v, raw2[i], 1));

    chart.setOption({
      animation: true,
      radar: {
        indicator: labels.map((name, i) => ({ name, max: maxes[i] })),
        center: ["50%", "55%"],
        radius: "52%",
        axisName: { color: "var(--color-text-secondary, #666)", fontSize: 10 },
        splitArea: { areaStyle: { color: ["rgba(22,119,255,0.02)", "rgba(22,119,255,0.04)"] } },
      },
      series: [{
        type: "radar",
        data: [
          {
            value: raw1,
            name: quote1.code,
            areaStyle: { color: "rgba(207,19,34,0.1)" },
            lineStyle: { color: "#cf1322" },
            itemStyle: { color: "#cf1322" },
          },
          {
            value: raw2,
            name: quote2.code,
            areaStyle: { color: "rgba(63,134,0,0.1)" },
            lineStyle: { color: "#3f8600" },
            itemStyle: { color: "#3f8600" },
          },
        ],
        symbol: "circle",
        symbolSize: 4,
      }],
    });

    const handleResize = () => chart.resize();
    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  }, [quote1, quote2, t]);

  return (
    <Card
      size="small"
      title={
        <span>
          <SwapOutlined /> {t("stockAnalysis.compare")}
        </span>
      }
      styles={{ body: { padding: "8px 10px" } }}
    >
      <div className="flex gap-1 mb-2 flex-wrap">
        <Input
          placeholder={t("stockAnalysis.stockCode1")}
          value={code1}
          onChange={(e) => setCode1(e.target.value)}
          style={{ width: 80 }}
          size="small"
        />
        <Input
          placeholder={t("stockAnalysis.stockCode2")}
          value={code2}
          onChange={(e) => setCode2(e.target.value)}
          style={{ width: 80 }}
          size="small"
        />
        <Button size="small" onClick={compare} disabled={!code1 || !code2} loading={loading}>
          {t("stockAnalysis.compareBtn")}
        </Button>
      </div>
      {quote1 && quote2 && (
        <>
          {/* 表格对比 */}
          <div className="flex items-center text-xs font-semibold mb-1" style={{ padding: "2px 0" }}>
            <span style={{ width: 48 }} />
            <span className="flex-1 text-center">
              {quote1.name}
              <br />
              {quote1.code}
            </span>
            <span className="flex-1 text-center">
              {quote2.name}
              <br />
              {quote2.code}
            </span>
          </div>
          <CompareRow
            label={t("stockAnalysis.price")}
            v1={quote1.price}
            v2={quote2.price}
          />
          <CompareRow
            label={t("stockAnalysis.change")}
            v1={`${quote1.changePct.toFixed(2)}%`}
            v2={`${quote2.changePct.toFixed(2)}%`}
            color1={quote1.changePct >= 0 ? "#cf1322" : "#3f8600"}
            color2={quote2.changePct >= 0 ? "#cf1322" : "#3f8600"}
          />
          <CompareRow label="PE" v1={quote1.pe ?? "-"} v2={quote2.pe ?? "-"} />
          <CompareRow label="PB" v1={quote1.pb ?? "-"} v2={quote2.pb ?? "-"} />
          <CompareRow
            label={t("stockAnalysis.volume")}
            v1={`${(quote1.volume / 10000).toFixed(1)}${t("stockAnalysis.volumeUnit")}`}
            v2={`${(quote2.volume / 10000).toFixed(1)}${t("stockAnalysis.volumeUnit")}`}
          />
          {/* 雷达图 */}
          <div ref={radarRef} style={{ width: "100%", height: 180, marginTop: 8 }} />
        </>
      )}
    </Card>
  );
}
