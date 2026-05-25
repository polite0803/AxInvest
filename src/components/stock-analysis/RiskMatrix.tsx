import { useStockAnalysisStore } from "@/stores";
import { Card, Tag } from "antd";
import * as echarts from "echarts";
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";

/** 风险类型 → 颜色映射（匹配后端 risk_type 字段，OKLch 值与 index.css --sa-* 同步） */
const RISK_COLORS: Record<string, string> = {
  "aggressive-debator": "oklch(55% 0.20 28)",
  "conservative-debator": "oklch(55% 0.18 150)",
  "neutral-debator": "oklch(55% 0.16 250)",
  "research-manager": "oklch(60% 0.18 85)",
  "comprehensive": "oklch(60% 0.16 290)",
};

/** 风险类型 → i18n key */
const RISK_LABEL_KEYS: Record<string, string> = {
  "aggressive-debator": "risk.aggressive",
  "conservative-debator": "risk.conservative",
  "neutral-debator": "risk.neutral",
  "research-manager": "risk.researchManager",
  "comprehensive": "risk.comprehensive",
};

/** 从风险评估文本中计算 0-100 的量化风险分 */
function computeRiskScore(text: string): number {
  const highRiskWords = [
    "高风险",
    "重大风险",
    "严重",
    "危机",
    "暴跌",
    "崩盘",
    "预警",
    "危险",
    "不确定",
    "大幅下",
    "极度",
  ];
  const midRiskWords = ["风险", "谨慎", "关注", "波动", "压力", "挑战", "不确定性", "潜在", "下行", "回落"];
  let score = 40; // 基准
  for (const w of highRiskWords) {
    const count = (text.match(new RegExp(w, "g")) || []).length;
    score += count * 8;
  }
  for (const w of midRiskWords) {
    const count = (text.match(new RegExp(w, "g")) || []).length;
    score += count * 3;
  }
  // 文本越长风险披露越充分 → 评分略增
  if (text.length > 500) { score += 5; }
  if (text.length > 1000) { score += 5; }
  if (text.length > 2000) { score += 5; }
  return Math.min(100, Math.max(5, score));
}

export function RiskMatrix() {
  const { t } = useTranslation();
  const riskAssessments = useStockAnalysisStore((s) => s.riskAssessments);
  const chartRef = useRef<HTMLDivElement>(null);
  const instanceRef = useRef<echarts.ECharts | null>(null);

  useEffect(() => {
    if (!chartRef.current) { return; }
    instanceRef.current = echarts.init(chartRef.current, undefined, { renderer: "canvas" });
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
    if (!chart || Object.keys(riskAssessments).length === 0) {
      chart?.clear();
      return;
    }

    const dimensions = Object.keys(riskAssessments).slice(0, 6).map((type) => {
      const key = RISK_LABEL_KEYS[type];
      return key ? t(`stockAnalysis.${key}`) : type;
    });

    const scores = Object.entries(riskAssessments).slice(0, 6).map(([, text]) => computeRiskScore(text));

    // 如果维度不足 3，不画雷达图
    if (dimensions.length < 3) { return; }

    chart.setOption({
      animation: true,
      animationDuration: 400,
      radar: {
        indicator: dimensions.map((name) => ({ name, max: 100 })),
        center: ["50%", "50%"],
        radius: "60%",
        axisName: { color: "var(--muted)", fontSize: 11 },
        splitArea: {
          areaStyle: {
            color: ["rgba(22,119,255,0.02)", "rgba(22,119,255,0.04)", "rgba(22,119,255,0.06)"],
          },
        },
        splitLine: { lineStyle: { color: "rgba(0,0,0,0.08)" } },
        axisLine: { lineStyle: { color: "rgba(0,0,0,0.08)" } },
      },
      series: [{
        type: "radar",
        data: [{ value: scores, name: t("stockAnalysis.riskAssessment") }],
        symbol: "circle",
        symbolSize: 6,
        areaStyle: { color: "oklch(55% 0.20 28 / 0.15)" },
        lineStyle: { color: "oklch(55% 0.20 28)", width: 2 },
        itemStyle: { color: "oklch(55% 0.20 28)" },
      }],
    });
  }, [riskAssessments, t]);

  if (Object.keys(riskAssessments).length === 0) { return null; }

  const entries = Object.entries(riskAssessments);

  return (
    <Card size="small" title={t("stockAnalysis.riskAssessment")} styles={{ body: { padding: 8 } }}>
      {/* 雷达图 */}
      {entries.length >= 3 && <div ref={chartRef} style={{ width: "100%", height: 220, marginBottom: 8 }} />}
      {/* 风险详情列表 */}
      <div className="flex flex-col gap-1.5">
        {entries.map(([type, report]) => {
          const color = RISK_COLORS[type]
            || `hsl(${type.split("").reduce((a, c) => a + c.charCodeAt(0), 0) % 360}, 50%, 45%)`;
          const label = RISK_LABEL_KEYS[type]
            ? t(`stockAnalysis.${RISK_LABEL_KEYS[type]}`)
            : type;
          const score = computeRiskScore(report);
          return (
            <div key={type} className="p-1.5 rounded" style={{ background: "var(--surface)" }}>
              <div className="text-sm font-medium mb-0.5 flex items-center justify-between">
                <Tag color={color} style={{ marginRight: 4 }}>{label}</Tag>
                <span
                  className="text-xs font-mono"
                  style={{ color: score > 70 ? "var(--sa-red)" : score > 40 ? "var(--sa-amber)" : "var(--sa-green)" }}
                >
                  {t("stockAnalysis.riskScore", { score })}
                </span>
              </div>
              <p
                className="text-xs leading-relaxed"
                style={{ whiteSpace: "pre-wrap", maxHeight: 120, overflow: "auto" }}
              >
                {report}
              </p>
            </div>
          );
        })}
      </div>
    </Card>
  );
}
