import { useSettingsStore, useStockAnalysisStore } from "@/stores";
import { DownloadOutlined, ExpandOutlined } from "@ant-design/icons";
import { Button, Card, Modal, Tag } from "antd";
import * as echarts from "echarts";
import NodeRenderer from "markstream-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { cleanToolCallTags } from "./utils";

/** 粗略检测文本是否看起来像 JSON */
function looksLikeJson(text: string): boolean {
  const trimmed = text.trim();
  return (trimmed.startsWith("{") && trimmed.endsWith("}"))
    || (trimmed.startsWith("[") && trimmed.endsWith("]"));
}

/**
 * 尝试从风险报告 JSON 中提取可读 Markdown 文本
 * 支持 a-risk / trader / portfolio-manager 等多种输出结构
 */
function extractReadableFromRiskReport(report: string): string {
  const cleaned = cleanToolCallTags(report);
  const trimmed = cleaned.trim();

  if (!looksLikeJson(trimmed)) { return cleaned; }

  try {
    const parsed = JSON.parse(trimmed);
    if (typeof parsed !== "object" || parsed === null) { return cleaned; }

    const parts: string[] = [];

    // 1. 立场/风格
    if (typeof parsed.stance === "string") {
      parts.push(`**立场**: ${parsed.stance}`);
    }

    // 2. 仓位/头寸
    if (typeof parsed.positionPct === "number") {
      parts.push(`**建议仓位**: ${parsed.positionPct}%`);
    }

    // 3. 信心度
    if (typeof parsed.confidence === "number") {
      parts.push(`**信心度**: ${parsed.confidence}%`);
    }

    // 4. 风险等级
    if (typeof parsed.riskLevel === "string") {
      parts.push(`**风险等级**: ${parsed.riskLevel}`);
    }

    // 5. 摘要/分析/推理
    const textFields = ["summary", "risk_analysis", "analysis", "reasoning", "report", "content", "text", "detail"];
    for (const field of textFields) {
      if (typeof parsed[field] === "string" && parsed[field].length > 5) {
        parts.push(parsed[field]);
      }
    }

    // 6. Kelly 公式参数
    if (parsed.kelly_inputs && typeof parsed.kelly_inputs === "object") {
      const k = parsed.kelly_inputs;
      const kParts: string[] = [];
      if (typeof k.win_rate === "number") { kParts.push(`胜率 ${(k.win_rate * 100).toFixed(0)}%`); }
      if (typeof k.payoff_ratio === "number") { kParts.push(`赔率 ${k.payoff_ratio}`); }
      if (typeof k.raw_kelly === "number") { kParts.push(`原始 Kelly ${(k.raw_kelly * 100).toFixed(1)}%`); }
      if (typeof k.scale_factor === "number") { kParts.push(`缩放因子 ${k.scale_factor}`); }
      if (kParts.length > 0) {
        parts.push(`**Kelly 参数**: ${kParts.join("，")}`);
      }
    }

    // 7. 非对称机会
    if (Array.isArray(parsed.asymmetric_opportunities) && parsed.asymmetric_opportunities.length > 0) {
      parts.push("**非对称机会**:");
      for (const opp of parsed.asymmetric_opportunities) {
        if (typeof opp.opportunity === "string") {
          parts.push(`- ${opp.opportunity}`);
        }
        if (Array.isArray(opp.evidence_refs)) {
          for (const ref of opp.evidence_refs) {
            if (typeof ref === "string") { parts.push(`  - 依据: ${ref}`); }
          }
        }
        if (typeof opp.expected_value === "string") {
          parts.push(`  - 预期价值: ${opp.expected_value}`);
        }
      }
    }

    // 8. 执行备注
    if (Array.isArray(parsed.execution_notes) && parsed.execution_notes.length > 0) {
      parts.push("**执行要点**:");
      for (const note of parsed.execution_notes) {
        if (typeof note === "string") { parts.push(`- ${note}`); }
      }
    } else if (typeof parsed.execution_notes === "string" && parsed.execution_notes.length > 5) {
      parts.push(`**执行要点**: ${parsed.execution_notes}`);
    }

    // 9. 风险项列表
    if (Array.isArray(parsed.risk_items) && parsed.risk_items.length > 0) {
      parts.push("**风险项**:");
      for (const item of parsed.risk_items) {
        if (typeof item.risk === "string") {
          const severity = typeof item.severity === "string" ? `（${item.severity}）` : "";
          parts.push(`- ${item.risk}${severity}`);
        }
        if (Array.isArray(item.evidence_refs)) {
          for (const ref of item.evidence_refs) {
            if (typeof ref === "string") { parts.push(`  - 依据: ${ref}`); }
          }
        }
      }
    }

    // 10. 关键条件跟踪
    if (Array.isArray(parsed.key_conditions_to_track) && parsed.key_conditions_to_track.length > 0) {
      parts.push("**关键跟踪条件**:");
      for (const cond of parsed.key_conditions_to_track) {
        if (typeof cond === "string") { parts.push(`- ${cond}`); }
      }
    }

    // 11. 多空核心论据
    if (Array.isArray(parsed.decisive_bull_acks) && parsed.decisive_bull_acks.length > 0) {
      parts.push("**核心做多论据**:");
      for (const ack of parsed.decisive_bull_acks) {
        if (typeof ack === "string") { parts.push(`- ${ack}`); }
      }
    }
    if (Array.isArray(parsed.decisive_bear_acks) && parsed.decisive_bear_acks.length > 0) {
      parts.push("**核心做空论据**:");
      for (const ack of parsed.decisive_bear_acks) {
        if (typeof ack === "string") { parts.push(`- ${ack}`); }
      }
    }

    // 12. 止损/止盈
    if (typeof parsed.stopLossPct === "number") {
      parts.push(`**止损**: -${parsed.stopLossPct}%`);
    }
    if (typeof parsed.takeProfitPct === "number") {
      parts.push(`**止盈**: +${parsed.takeProfitPct}%`);
    }

    if (parts.length > 0) {
      return parts.join("\n\n");
    }

    // 兜底：提取所有字符串值
    for (const [key, value] of Object.entries(parsed)) {
      if (typeof value === "string" && value.length > 10) {
        parts.push(`**${key}**: ${value}`);
      }
    }
    if (parts.length > 0) { return parts.join("\n\n"); }
  } catch {
    // 解析失败回退
  }

  return cleaned;
}

/** 风险类型 → 颜色映射（键名对齐 riskAssessments 实际节点 ID，
 *  OKLch 值与 index.css --sa-* 同步）。
 * 修复 Bug #5: 旧版键名用专家角色 ID（aggressive-debator 等），
 * 与 store 中 riskAssessments 实际键（risk-agg 等）不匹配，导致颜色与标签
 * 全部回退到默认值，用户看到的是 raw 节点 ID + 随机 HSL 颜色。
 */
const RISK_COLORS: Record<string, string> = {
  "risk-agg": "oklch(55% 0.20 28)",
  "risk-con": "oklch(55% 0.18 150)",
  "risk-neu": "oklch(55% 0.16 250)",
  "research-mgr": "oklch(60% 0.18 85)",
  "comprehensive": "oklch(60% 0.16 290)",
  "risk-aggregated": "oklch(55% 0.20 28)",
  "risk-level": "oklch(55% 0.18 45)",
};

/** 风险类型 → i18n key（键名对齐 riskAssessments 实际节点 ID） */
const RISK_LABEL_KEYS: Record<string, string> = {
  "risk-agg": "stockAnalysis.risk.aggressive",
  "risk-con": "stockAnalysis.risk.conservative",
  "risk-neu": "stockAnalysis.risk.neutral",
  "research-mgr": "stockAnalysis.risk.researchManager",
  "comprehensive": "stockAnalysis.risk.comprehensive",
  "risk-aggregated": "stockAnalysis.workflow.riskAggregation",
  "risk-level": "stockAnalysis.riskLevel",
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

/** 把风险评估条目序列化为可分享的 Markdown 文档 */
function buildRiskMarkdown(
  entries: Array<[string, string]>,
  t: (key: string, opts?: Record<string, unknown>) => string,
  stockCode?: string,
  stockName?: string,
): string {
  const now = new Date().toISOString().replace("T", " ").substring(0, 19);
  const title = stockName && stockCode
    ? `${stockName} (${stockCode}) — 风险评估报告`
    : "风险评估报告";
  const lines: string[] = [
    `# ${title}`,
    "",
    `> 导出时间：${now}  `,
    `> 评估维度：${entries.length}`,
    "",
    "## 风险评分总览",
    "",
    "| 维度 | 风险评分 |",
    "| --- | --- |",
    ...entries.map(([type, report]) => {
      const label = RISK_LABEL_KEYS[type] ? t(RISK_LABEL_KEYS[type]) : type;
      const score = computeRiskScore(report);
      return `| ${label} | ${score} / 100 |`;
    }),
    "",
    "## 详细评估",
    "",
  ];
  for (const [type, report] of entries) {
    const label = RISK_LABEL_KEYS[type] ? t(RISK_LABEL_KEYS[type]) : type;
    const score = computeRiskScore(report);
    const readable = extractReadableFromRiskReport(report) || t("stockAnalysis.noRiskData");
    lines.push(
      `### ${label}（${score} / 100）`,
      "",
      readable,
      "",
      "---",
      "",
    );
  }
  lines.push(
    `*本文档由 AxInvest 风险评估模块自动生成，仅供研究参考，不构成投资建议。*`,
  );
  return lines.join("\n");
}

export function RiskMatrix() {
  const { t } = useTranslation();
  const themeMode = useSettingsStore((s) => s.settings.theme_mode);
  const isDark = themeMode === "dark"
    || (themeMode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
  const riskAssessments = useStockAnalysisStore((s) => s.riskAssessments);
  const stockCode = useStockAnalysisStore((s) => s.stockCode);
  const stockName = useStockAnalysisStore((s) => s.stockName);
  const [chartReady, setChartReady] = useState(false);
  const chartRef = useRef<HTMLDivElement>(null);
  const instanceRef = useRef<echarts.ECharts | null>(null);
  const [expanded, setExpanded] = useState(false);
  const [selectedCard, setSelectedCard] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);
  const expandedChartRef = useRef<HTMLDivElement>(null);
  const expandedInstanceRef = useRef<echarts.ECharts | null>(null);

  /** 大屏模式一键导出风险评估为 Markdown 文档 */
  const handleExportRiskMarkdown = async () => {
    const entries = Object.entries(riskAssessments);
    if (entries.length === 0 || exporting) { return; }
    setExporting(true);
    try {
      const md = buildRiskMarkdown(entries, t, stockCode, stockName);
      const { saveFileMarkdown } = await import("@/lib/exportChat");
      const safeName = (stockName && stockCode)
        ? `${stockName}-${stockCode}-风险评估`
        : "风险评估";
      await saveFileMarkdown(safeName, md);
    } finally {
      setExporting(false);
    }
  };

  useEffect(() => {
    if (!chartRef.current) { return; }
    if (chartRef.current.clientWidth === 0 || chartRef.current.clientHeight === 0) {
      const timer = requestAnimationFrame(() => setChartReady((v) => !v ? v : true));
      return () => cancelAnimationFrame(timer);
    }
    instanceRef.current = echarts.init(chartRef.current, undefined, { renderer: "canvas" });
    const chart = instanceRef.current;
    const handleResize = () => {
      if (!chart.isDisposed()) { chart.resize(); }
    };
    window.addEventListener("resize", handleResize);
    return () => {
      window.removeEventListener("resize", handleResize);
      chart.dispose();
      instanceRef.current = null;
    };
  }, [chartReady]);

  useEffect(() => {
    const chart = instanceRef.current;
    if (!chart || chart.isDisposed()) { return; }
    if (Object.keys(riskAssessments).length === 0) {
      chart.clear();
      return;
    }

    const dimensions = Object.keys(riskAssessments).slice(0, 6).map((type) => {
      const key = RISK_LABEL_KEYS[type];
      return key ? t(key) : type;
    });

    const scores = Object.entries(riskAssessments).slice(0, 6).map(([, text]) => computeRiskScore(text));

    // 如果维度不足 3，不画雷达图
    if (dimensions.length < 3) { return; }

    // 深色主题下使用高对比度颜色
    const axisTextColor = isDark ? "rgba(255,255,255,0.6)" : "rgba(0,0,0,0.5)";
    const lineColor = isDark ? "rgba(255,255,255,0.12)" : "rgba(0,0,0,0.08)";
    const areaColors = isDark
      ? ["rgba(82,130,255,0.06)", "rgba(82,130,255,0.10)", "rgba(82,130,255,0.14)"]
      : ["rgba(22,119,255,0.02)", "rgba(22,119,255,0.04)", "rgba(22,119,255,0.06)"];

    chart.setOption({
      animation: true,
      animationDuration: 400,
      radar: {
        indicator: dimensions.map((name) => ({ name, max: 100 })),
        center: ["50%", "50%"],
        radius: "60%",
        axisName: { color: axisTextColor, fontSize: 11 },
        splitArea: {
          areaStyle: {
            color: areaColors,
          },
        },
        splitLine: { lineStyle: { color: lineColor } },
        axisLine: { lineStyle: { color: lineColor } },
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
  }, [riskAssessments, t, isDark]);

  // Render expanded chart when modal opens
  useEffect(() => {
    if (!expanded || !expandedChartRef.current || Object.keys(riskAssessments).length === 0) {
      expandedInstanceRef.current?.dispose();
      expandedInstanceRef.current = null;
      return;
    }
    if (expandedChartRef.current.clientWidth === 0 || expandedChartRef.current.clientHeight === 0) {
      const timer = requestAnimationFrame(() => setExpanded((v) => v));
      return () => cancelAnimationFrame(timer);
    }
    expandedInstanceRef.current?.dispose();
    const chart = echarts.init(expandedChartRef.current, undefined, { renderer: "canvas" });
    expandedInstanceRef.current = chart;
    const dimensions = Object.keys(riskAssessments).slice(0, 6).map((type) => {
      const key = RISK_LABEL_KEYS[type];
      return key ? t(key) : type;
    });
    const scores = Object.entries(riskAssessments).slice(0, 6).map(([, text]) => computeRiskScore(text));
    if (dimensions.length >= 3) {
      const axisTextColor = isDark ? "rgba(255,255,255,0.6)" : "rgba(0,0,0,0.5)";
      const lineColor = isDark ? "rgba(255,255,255,0.12)" : "rgba(0,0,0,0.08)";
      const areaColors = isDark
        ? ["rgba(82,130,255,0.06)", "rgba(82,130,255,0.10)", "rgba(82,130,255,0.14)"]
        : ["rgba(22,119,255,0.02)", "rgba(22,119,255,0.04)", "rgba(22,119,255,0.06)"];

      chart.setOption({
        animation: true,
        radar: {
          indicator: dimensions.map((name) => ({ name, max: 100 })),
          center: ["50%", "50%"],
          radius: "65%",
          axisName: { color: axisTextColor, fontSize: 13 },
          splitArea: {
            areaStyle: { color: areaColors },
          },
          splitLine: { lineStyle: { color: lineColor } },
          axisLine: { lineStyle: { color: lineColor } },
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
    }
    const handleResize = () => {
      if (!chart.isDisposed()) { chart.resize(); }
    };
    window.addEventListener("resize", handleResize);
    return () => {
      window.removeEventListener("resize", handleResize);
      chart.dispose();
      expandedInstanceRef.current = null;
    };
  }, [expanded, riskAssessments, t]);

  if (Object.keys(riskAssessments).length === 0) { return null; }

  const entries = Object.entries(riskAssessments);

  return (
    <>
      <Card
        size="small"
        title={t("stockAnalysis.riskAssessment")}
        extra={
          <Button
            type="text"
            size="small"
            icon={<ExpandOutlined />}
            onClick={() => setExpanded(true)}
          />
        }
        styles={{ body: { padding: 8 } }}
      >
        {entries.length >= 3 && <div ref={chartRef} style={{ width: "100%", height: 220, marginBottom: 8 }} />}
        <div className="flex flex-col gap-1.5">
          {entries.map(([type, report]) => {
            const color = RISK_COLORS[type]
              || `hsl(${type.split("").reduce((a, c) => a + c.charCodeAt(0), 0) % 360}, 50%, 45%)`;
            const label = RISK_LABEL_KEYS[type]
              ? t(RISK_LABEL_KEYS[type])
              : type;
            const score = computeRiskScore(report);
            return (
              <div key={type} className="p-1.5 rounded" style={{ background: "var(--surface)" }}>
                <div className="text-sm font-medium mb-0.5 flex items-center justify-between">
                  <div className="flex items-center gap-1">
                    <Tag color={color} style={{ marginRight: 4 }}>{label}</Tag>
                    <Button
                      type="text"
                      size="small"
                      icon={<ExpandOutlined />}
                      style={{ padding: 0, width: 20, height: 20, fontSize: 10 }}
                      onClick={() => setSelectedCard(type)}
                      title="展开详情"
                    />
                  </div>
                  <span
                    className="text-xs font-mono"
                    style={{ color: score > 70 ? "var(--sa-red)" : score > 40 ? "var(--sa-amber)" : "var(--sa-green)" }}
                  >
                    {t("stockAnalysis.riskScore", { score })}
                  </span>
                </div>
                <div
                  className="sa-markdown-content text-xs leading-relaxed"
                  style={{ maxHeight: 160, overflow: "auto" }}
                >
                  {(() => {
                    const readable = extractReadableFromRiskReport(report);
                    return readable
                      ? <NodeRenderer content={readable} isDark={isDark} />
                      : <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.noRiskData")}</span>;
                  })()}
                </div>
              </div>
            );
          })}
        </div>
      </Card>

      <Modal
        title={t("stockAnalysis.riskAssessment")}
        open={expanded}
        onCancel={() => setExpanded(false)}
        footer={[
          <Button
            key="export"
            type="primary"
            icon={<DownloadOutlined />}
            loading={exporting}
            onClick={handleExportRiskMarkdown}
            disabled={entries.length === 0}
          >
            {t("common.exportMarkdown", { defaultValue: "导出 Markdown" })}
          </Button>,
        ]}
        width="80vw"
        style={{ top: 20 }}
        styles={{ body: { maxHeight: "80vh", overflow: "auto" } }}
      >
        {entries.length >= 3 && <div ref={expandedChartRef} style={{ width: "100%", height: 320, marginBottom: 16 }} />}
        <div className="flex flex-col gap-3">
          {entries.map(([type, report]) => {
            const color = RISK_COLORS[type]
              || `hsl(${type.split("").reduce((a, c) => a + c.charCodeAt(0), 0) % 360}, 50%, 45%)`;
            const label = RISK_LABEL_KEYS[type]
              ? t(RISK_LABEL_KEYS[type])
              : type;
            const score = computeRiskScore(report);
            return (
              <div key={type} className="p-3 rounded" style={{ background: "var(--surface)" }}>
                <div className="text-base font-medium mb-1 flex items-center justify-between">
                  <Tag color={color}>{label}</Tag>
                  <span
                    className="font-mono"
                    style={{ color: score > 70 ? "var(--sa-red)" : score > 40 ? "var(--sa-amber)" : "var(--sa-green)" }}
                  >
                    {t("stockAnalysis.riskScore", { score })}
                  </span>
                </div>
                <div className="sa-markdown-content text-sm leading-relaxed">
                  {(() => {
                    const readable = extractReadableFromRiskReport(report);
                    return readable
                      ? <NodeRenderer content={readable} isDark={isDark} />
                      : <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.noRiskData")}</span>;
                  })()}
                </div>
              </div>
            );
          })}
        </div>
      </Modal>

      {/* 单个风险卡片大屏详情 Modal */}
      <Modal
        title={(() => {
          const type = selectedCard;
          if (!type) { return ""; }
          const label = RISK_LABEL_KEYS[type] ? t(RISK_LABEL_KEYS[type]) : type;
          const color = RISK_COLORS[type]
            || `hsl(${type.split("").reduce((a, c) => a + c.charCodeAt(0), 0) % 360}, 50%, 45%)`;
          const score = type ? computeRiskScore(riskAssessments[type] || "") : 0;
          return (
            <div className="flex items-center gap-2">
              <Tag color={color}>{label}</Tag>
              <span
                className="font-mono text-sm"
                style={{ color: score > 70 ? "var(--sa-red)" : score > 40 ? "var(--sa-amber)" : "var(--sa-green)" }}
              >
                {t("stockAnalysis.riskScore", { score })}
              </span>
            </div>
          );
        })()}
        open={!!selectedCard}
        onCancel={() => setSelectedCard(null)}
        footer={null}
        width="70vw"
        style={{ top: 40 }}
        styles={{ body: { maxHeight: "75vh", overflow: "auto", padding: 24 } }}
      >
        {selectedCard && riskAssessments[selectedCard] && (
          <div className="sa-markdown-content leading-relaxed text-base">
            {(() => {
              const readable = extractReadableFromRiskReport(riskAssessments[selectedCard]);
              return readable
                ? <NodeRenderer content={readable} isDark={isDark} />
                : <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.noRiskData")}</span>;
            })()}
          </div>
        )}
      </Modal>
    </>
  );
}
