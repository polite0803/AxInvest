import { useSettingsStore } from "@/stores";
import { getSignalColor } from "@/lib/stock-analysis-utils";
import { ExpandOutlined } from "@ant-design/icons";
import { Button, Card, Collapse, Empty, Modal, Tag } from "antd";
import NodeRenderer from "markstream-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { cleanToolCallTags, tryBeautifyJson } from "./utils";

interface Props {
  expertId: string;
  report: string;
}

/** 宽松解析：支持多种后端 JSON 格式 */
interface ParsedReport {
  // 标准格式
  type?: string;
  summary?: string;
  signals?: string[];
  risk_flags?: string[];
  argument?: string;
  key_points?: string[];
  confidence?: number;
  // 辩论格式
  core_arguments?: string[];
  resonance_points?: string[];
  preempted_counter_attacks?: string[];
  bull_strength_score?: number;
  bear_strength_score?: number;
  data_gaps?: string[];
  // 资金面/筹码面格式
  main_flow_state?: string;
  active_player?: string;
  dragon_tiger_signal?: string;
  limit_up_sustainability?: string;
  bull_score?: number;
  bear_score?: number;
  trigger_bull?: string;
  trigger_bear?: string;
  evidence?: Array<{ point?: string; data?: string; weight?: number }>;
  // 估值格式
  expert?: string;
  business_model?: string;
  moat_rating?: string;
  moat_reasoning?: string;
  financial_health?: string;
  intrinsic_value_range?: string | null;
  margin_of_safety?: string;
  buffett_verdict?: string;
  ideal_buy_price?: string | null;
  // 通用分析
  analysis?: string;
  assessment?: string;
  verdict?: string;
  reasoning?: string;
  stance?: string;
  positionPct?: number;
  action?: string;
  [key: string]: unknown;
}

function tryParse(report: string): ParsedReport | null {
  try {
    const trimmed = report.trim();
    // 1) 直接是 JSON
    if (trimmed.startsWith("{") || trimmed.startsWith("[")) {
      try {
        return JSON.parse(trimmed);
      } catch { /* try below */ }
    }
    // 2) ```json ``` 代码块
    const m = trimmed.match(/```(?:json)?\s*([\s\S]*?)\s*```/);
    if (m) {
      try {
        return JSON.parse(m[1].trim());
      } catch { /* try below */ }
    }
    // 3) 复用 tryBeautifyJson 的容错提取（处理前面有解释文字、trailing comma 等）
    const beautified = tryBeautifyJson(report);
    if (beautified !== report) {
      // tryBeautifyJson 返回了不同的字符串，说明它做了提取/修复
      try {
        return JSON.parse(beautified);
      } catch { /* fallthrough */ }
    }
    // 4) 手动找第一个 { 到最后一个 }
    const fb = trimmed.indexOf("{");
    const lb = trimmed.lastIndexOf("}");
    if (fb !== -1 && lb !== -1 && lb > fb) {
      try {
        return JSON.parse(trimmed.slice(fb, lb + 1));
      } catch { /* ignore */ }
    }
  } catch { /* not JSON */ }
  return null;
}

/** 从任意 ParsedReport 中提取可读的摘要文本 */
function extractSummary(parsed: ParsedReport): string {
  const candidates = [
    parsed.summary,
    parsed.argument,
    parsed.analysis,
    parsed.assessment,
    parsed.buffett_verdict,
    parsed.verdict,
    parsed.reasoning,
    parsed.business_model,
    parsed.moat_reasoning,
    parsed.financial_health,
    parsed.margin_of_safety,
  ];
  for (const c of candidates) {
    if (typeof c === "string" && c.length > 10) { return c; }
  }
  return "";
}

/** 从 ParsedReport 中提取标签列表 */
function extractTags(parsed: ParsedReport): string[] {
  if (Array.isArray(parsed.signals) && parsed.signals.length > 0) {
    return parsed.signals;
  }
  const tags: string[] = [];
  if (parsed.stance) { tags.push(parsed.stance); }
  if (parsed.action) { tags.push(parsed.action); }
  if (parsed.main_flow_state) { tags.push(`资金流:${parsed.main_flow_state}`); }
  if (parsed.dragon_tiger_signal) { tags.push(parsed.dragon_tiger_signal); }
  if (parsed.moat_rating) { tags.push(`护城河:${parsed.moat_rating}`); }
  if (typeof parsed.bull_score === "number" && parsed.bull_score > 0) {
    // 归一化到百分制：十分制(≤10) ×10，百分制(>10) 不处理
    const normalized = parsed.bull_score <= 10 ? parsed.bull_score * 10 : parsed.bull_score;
    tags.push(`看多:${Math.round(normalized)}`);
  }
  if (typeof parsed.bear_score === "number" && parsed.bear_score > 0) {
    const normalized = parsed.bear_score <= 10 ? parsed.bear_score * 10 : parsed.bear_score;
    tags.push(`看空:${Math.round(normalized)}`);
  }
  return tags;
}

/** 从 ParsedReport 中提取要点列表 */
function extractKeyPoints(parsed: ParsedReport): string[] {
  if (Array.isArray(parsed.key_points) && parsed.key_points.length > 0) {
    return parsed.key_points;
  }
  if (Array.isArray(parsed.core_arguments) && parsed.core_arguments.length > 0) {
    return parsed.core_arguments;
  }
  if (Array.isArray(parsed.resonance_points) && parsed.resonance_points.length > 0) {
    return parsed.resonance_points;
  }
  if (Array.isArray(parsed.evidence) && parsed.evidence.length > 0) {
    return parsed.evidence
      .filter((e) => e && typeof e.point === "string")
      .map((e) => `${e.point}${e.data ? ` (${e.data})` : ""}`);
  }
  if (Array.isArray(parsed.data_gaps) && parsed.data_gaps.length > 0) {
    return parsed.data_gaps.slice(0, 3);
  }
  return [];
}

/** 从 ParsedReport 中提取风险标志 */
function extractRiskFlags(parsed: ParsedReport): string[] {
  if (Array.isArray(parsed.risk_flags) && parsed.risk_flags.length > 0) {
    return parsed.risk_flags;
  }
  return [];
}

/** 判断是否为"空分析"（全是 data_gaps 或空字段） */
function isEmptyAnalysis(parsed: ParsedReport): boolean {
  const summary = extractSummary(parsed);
  if (summary.length > 20) { return false; }
  const points = extractKeyPoints(parsed);
  if (points.length > 0) { return false; }
  // 如果只有 data_gaps 且没有实质内容，认为是空的
  const hasContent = Object.keys(parsed).some((k) => {
    const v = parsed[k];
    if (k === "data_gaps") { return false; }
    if (Array.isArray(v)) { return v.length > 0; }
    if (typeof v === "string") { return v.length > 5 && !v.includes("无法"); }
    if (typeof v === "number") { return v !== 0; }
    return v != null;
  });
  return !hasContent;
}

export function AnalystReportCard({ expertId, report }: Props) {
  const { t } = useTranslation();
  const themeMode = useSettingsStore((s) => s.settings.theme_mode);
  const isDark = themeMode === "dark"
    || (themeMode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
  const name = t(`stockAnalysis.workflow.analyst.${expertId}`, expertId);
  const cleanedReport = cleanToolCallTags(report);
  const beautified = tryBeautifyJson(cleanedReport);
  const parsed = tryParse(beautified);
  const displayContent = beautified || report;
  const [expanded, setExpanded] = useState(false);
  const hasContent = !!displayContent || !!parsed;

  const expandBtn = hasContent
    ? (
      <Button
        type="text"
        size="small"
        icon={<ExpandOutlined />}
        onClick={() => setExpanded(true)}
        title={t("stockAnalysis.expandView")}
      />
    )
    : null;

  // 有解析结果：尝试结构化渲染
  if (parsed) {
    const summary = extractSummary(parsed);
    const tags = extractTags(parsed);
    const points = extractKeyPoints(parsed);
    const riskFlags = extractRiskFlags(parsed);
    const empty = isEmptyAnalysis(parsed);
    const confidence = typeof parsed.confidence === "number"
      ? (parsed.confidence > 1 ? parsed.confidence : parsed.confidence * 100)
      : (typeof parsed.positionPct === "number" ? parsed.positionPct : null);

    return (
      <>
        <Card
          size="small"
          className="h-full flex flex-col"
          title={
            <span>
              {name}
              {parsed.type && <Tag style={{ marginLeft: 8 }}>{parsed.type}</Tag>}
              {empty && <Tag color="orange" style={{ marginLeft: 8 }}>数据不足</Tag>}
            </span>
          }
          extra={expandBtn}
          styles={{ body: { flex: 1, maxHeight: 400, overflow: "auto" } }}
        >
          {summary && (
            <div className="sa-markdown-content text-xs">
              <NodeRenderer content={summary} isDark={isDark} />
            </div>
          )}
          {points.length > 0 && (
            <ul className="text-xs list-disc pl-4 mb-1 mt-1" style={{ color: "var(--muted)" }}>
              {points.map((p, i) => <li key={i}>{p}</li>)}
            </ul>
          )}
          {tags.length > 0 && (
            <div className="flex gap-1 flex-wrap mt-1">
              {tags.map((s, i) => (
                <Tag key={i} color={getSignalColor(s)}>
                  {s}
                </Tag>
              ))}
            </div>
          )}
          {riskFlags.length > 0 && (
            <div className="flex gap-1 flex-wrap mt-1">
              {riskFlags.map((r, i) => <Tag key={i} color="orange">{r}</Tag>)}
            </div>
          )}
          {confidence != null && (
            <div className="text-xs mt-1" style={{ color: "var(--muted)" }}>
              {t("stockAnalysis.confidence")}: {confidence.toFixed(0)}%
            </div>
          )}
          {!summary && points.length === 0 && tags.length === 0 && (
            <div className="text-xs" style={{ color: "var(--muted)" }}>
              分析完成，但未返回结构化内容
            </div>
          )}
        </Card>
        <Modal
          title={name}
          open={expanded}
          onCancel={() => setExpanded(false)}
          footer={null}
          width="80vw"
          style={{ top: 20 }}
          styles={{ body: { maxHeight: "80vh", overflow: "auto" } }}
        >
          <div className="sa-markdown-content">
            {summary && <NodeRenderer content={summary} isDark={isDark} />}
            {points.length > 0 && (
              <ul className="list-disc pl-6 my-2">
                {points.map((p, i) => <li key={i}>{p}</li>)}
              </ul>
            )}
            <Collapse
              defaultActiveKey={[]}
              items={[{
                key: "raw",
                label: "查看原始数据",
                children: (
                  <pre
                    style={{
                      background: isDark ? "#1f1f1f" : "#f5f5f5",
                      padding: 12,
                      borderRadius: 6,
                      fontSize: 12,
                      overflow: "auto",
                      maxHeight: "50vh",
                      margin: 0,
                    }}
                  >
                    <code>{displayContent}</code>
                  </pre>
                ),
              }]}
              style={{ marginTop: 12 }}
            />
          </div>
        </Modal>
      </>
    );
  }

  // 无解析结果：尝试从原始文本中"模糊"提取关键信息
  const fuzzy = ((): { points: string[]; tags: string[]; summary: string; empty: boolean } => {
    const text = displayContent || "";
    const points: string[] = [];
    const tags: string[] = [];
    let summary = "";

    // 提取 evidence[].point
    const evMatch = text.match(/"evidence"\s*:\s*\[([\s\S]*?)\]/);
    if (evMatch) {
      const items = evMatch[1].match(/"point"\s*:\s*"([^"]*)"/g);
      if (items) {
        items.forEach((item) => {
          const m = item.match(/"point"\s*:\s*"([^"]*)"/);
          if (m && m[1].length > 3) { points.push(m[1]); }
        });
      }
    }

    // 提取 core_arguments[]
    const coreMatch = text.match(/"core_arguments"\s*:\s*\[([\s\S]*?)\]/);
    if (coreMatch) {
      const raw = coreMatch[1];
      // 匹配 "string" 或 { "point": "..." }
      const strItems = raw.match(/"([^"\n]{5,})"/g);
      if (strItems) {
        strItems.forEach((item) => {
          const clean = item.replace(/^"|"$/g, "");
          if (clean.length > 5 && !points.includes(clean)) { points.push(clean); }
        });
      }
      const objItems = raw.match(/"point"\s*:\s*"([^"]*)"/g);
      if (objItems) {
        objItems.forEach((item) => {
          const m = item.match(/"point"\s*:\s*"([^"]*)"/);
          if (m && m[1].length > 3 && !points.includes(m[1])) { points.push(m[1]); }
        });
      }
    }

    // 提取 key_points[]
    const kpMatch = text.match(/"key_points"\s*:\s*\[([\s\S]*?)\]/);
    if (kpMatch) {
      const items = kpMatch[1].match(/"([^"\n]{5,})"/g);
      if (items) {
        items.forEach((item) => {
          const clean = item.replace(/^"|"$/g, "");
          if (clean.length > 5 && !points.includes(clean)) { points.push(clean); }
        });
      }
    }

    // 提取 data_gaps（作为数据不足提示）
    const gapMatch = text.match(/"data_gaps"\s*:\s*\[([\s\S]*?)\]/);
    if (gapMatch && points.length === 0) {
      const items = gapMatch[1].match(/"([^"\n]{5,})"/g);
      if (items && items.length > 0) {
        const first = items[0].replace(/^"|"$/g, "");
        if (first.length > 5) { summary = `数据受限：${first}`; }
      }
    }

    // 提取 bull_score / bear_score（归一化到百分制）
    const bullMatch = text.match(/"bull_score"\s*:\s*(\d+)/);
    const bearMatch = text.match(/"bear_score"\s*:\s*(\d+)/);
    if (bullMatch) {
      const raw = parseInt(bullMatch[1], 10);
      const normalized = raw <= 10 ? raw * 10 : raw;
      tags.push(`看多:${Math.round(normalized)}`);
    }
    if (bearMatch) {
      const raw = parseInt(bearMatch[1], 10);
      const normalized = raw <= 10 ? raw * 10 : raw;
      tags.push(`看空:${Math.round(normalized)}`);
    }

    // 提取 stance / action
    const stanceMatch = text.match(/"stance"\s*:\s*"([^"]*)"/);
    if (stanceMatch) { tags.push(stanceMatch[1]); }
    const actionMatch = text.match(/"action"\s*:\s*"([^"]*)"/);
    if (actionMatch) { tags.push(actionMatch[1]); }

    // 提取 summary
    const summaryMatch = text.match(/"summary"\s*:\s*"([^"]*)"/);
    if (summaryMatch && summaryMatch[1].length > 10) {
      summary = summaryMatch[1];
    }
    // 提取 argument
    if (!summary) {
      const argMatch = text.match(/"argument"\s*:\s*"([^"]*)"/);
      if (argMatch && argMatch[1].length > 10) { summary = argMatch[1]; }
    }
    // 提取 analysis
    if (!summary) {
      const anaMatch = text.match(/"analysis"\s*:\s*"([^"]*)"/);
      if (anaMatch && anaMatch[1].length > 10) { summary = anaMatch[1]; }
    }

    // 纯文本 fallback：内容不像 JSON 但有实质文字，提取前 200 字作为摘要
    if (!summary && points.length === 0 && tags.length === 0) {
      const cleaned = text
        .replace(/由于上游工具调用返回了.*?的错误[，。]/g, "")
        .replace(/根据系统指令.*?[，。]/g, "")
        .replace(/我的职责是.*?[，。]/g, "")
        .replace(/在上游数据缺失.*?[，。]/g, "")
        .replace(/我无法获取.*?[，。]/g, "")
        .replace(/我必须诚实反映.*?[，。]/g, "")
        .replace(/以下是基于当前可用上下文.*?[，。]/g, "")
        .replace(/请注意，由于缺乏.*?[，。]/g, "")
        .replace(/我需要调用.*?[，。]/g, "")
        .replace(/让我先调用.*?[，。]/g, "")
        .replace(/首先我需要.*?[。]/g, "")
        .replace(/用户要求.*?进行.*?[。]/g, "")
        .replace(/用户需要.*?[。]/g, "")
        .replace(/^\s*[-*]\s+/gm, "") // markdown list
        .replace(/\s+/g, " ")
        .trim();
      if (cleaned.length > 30) {
        summary = cleaned.slice(0, 200) + (cleaned.length > 200 ? "..." : "");
      } else if (!text.trim().startsWith("{") && !text.trim().startsWith("[") && text.length > 30) {
        summary = text.slice(0, 200) + "...";
      }
    }

    const empty = points.length === 0 && !summary && tags.length === 0;
    return { points, tags, summary, empty };
  })();

  return (
    <>
      <Card
        size="small"
        className="h-full flex flex-col"
        title={
          <span>
            {name}
            {fuzzy.empty && <Tag color="orange" style={{ marginLeft: 8 }}>数据不足</Tag>}
          </span>
        }
        extra={expandBtn}
        styles={{ body: { flex: 1, maxHeight: 400, overflow: "auto" } }}
      >
        {fuzzy.summary && (
          <div className="sa-markdown-content text-xs">
            <NodeRenderer content={fuzzy.summary} isDark={isDark} />
          </div>
        )}
        {fuzzy.points.length > 0 && (
          <ul className="text-xs list-disc pl-4 mb-1 mt-1" style={{ color: "var(--muted)" }}>
            {fuzzy.points.map((p, i) => <li key={i}>{p}</li>)}
          </ul>
        )}
        {fuzzy.tags.length > 0 && (
          <div className="flex gap-1 flex-wrap mt-1">
            {fuzzy.tags.map((s, i) => (
              <Tag key={i} color={getSignalColor(s)}>
                {s}
              </Tag>
            ))}
          </div>
        )}
        {fuzzy.empty && displayContent && (
          <div className="text-xs" style={{ color: "var(--muted)" }}>
            数据不足
          </div>
        )}
        {!displayContent && (
          <Empty
            image={Empty.PRESENTED_IMAGE_SIMPLE}
            description={
              <span style={{ color: "var(--muted)", fontSize: 12 }}>
                {t("stockAnalysis.noReport")}
              </span>
            }
          />
        )}
      </Card>
      <Modal
        title={name}
        open={expanded}
        onCancel={() => setExpanded(false)}
        footer={null}
        width="80vw"
        style={{ top: 20 }}
        styles={{ body: { maxHeight: "80vh", overflow: "auto" } }}
      >
        <div className="sa-markdown-content">
          {fuzzy.summary && <NodeRenderer content={fuzzy.summary} isDark={isDark} />}
          {fuzzy.points.length > 0 && (
            <ul className="list-disc pl-6 my-2">
              {fuzzy.points.map((p, i) => <li key={i}>{p}</li>)}
            </ul>
          )}
          <Collapse
            defaultActiveKey={[]}
            items={[{
              key: "raw",
              label: "查看原始数据",
              children: (
                <pre
                  style={{
                    background: isDark ? "#1f1f1f" : "#f5f5f5",
                    padding: 12,
                    borderRadius: 6,
                    fontSize: 12,
                    overflow: "auto",
                    maxHeight: "50vh",
                    margin: 0,
                  }}
                >
                  <code>{displayContent}</code>
                </pre>
              ),
            }]}
            style={{ marginTop: 12 }}
          />
        </div>
      </Modal>
    </>
  );
}
