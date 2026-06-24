import { classifySentiment } from "@/lib/stock-analysis-utils";
import { useSettingsStore, useStockAnalysisStore } from "@/stores";
import { ExpandOutlined, ReloadOutlined, WarningOutlined } from "@ant-design/icons";
import { Alert, Button, Card, Empty, Modal, Segmented, Tag, Typography } from "antd";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { cleanToolCallTags, tryBeautifyJson } from "./utils";

/* ─── 类型定义 ─── */

interface DebateContent {
  text: string; // 清理后的原始文本（JSON 或 Markdown）
  parsed: DebateJson | null; // 结构化解析结果
  empty: boolean; // 是否为空
}

/** R1 辩手输出格式 */
interface R1DebateJson {
  core_arguments?: Array<{
    claim?: string;
    category?: string;
    evidence_refs?: string[];
    strength?: number;
    timeliness?: string;
  }>;
  resonance_points?: Array<{
    point?: string;
    dimensions?: string[];
    weight?: number;
  }>;
  preempted_counter_attacks?: Array<{
    our_claim?: string;
    bear_attack?: string;
    our_response?: string;
  }>;
  bull_strength_score?: number;
  bear_strength_score?: number;
  data_gaps?: string[];
}

/** R2 质询输出格式 */
interface R2DebateJson {
  cross_examination?: Array<{
    target_claim_ref?: string;
    weakness_type?: string;
    questions?: string[];
    if_unanswered_impact?: string;
  }>;
  summary_for_convergence?: string;
}

/** R3 最终反驳输出格式 */
interface R3DebateJson {
  final_position?: string;
  claim?: string;
  confidence?: number;
  r2_cross_examination_response?: Array<{
    r2_question_ref?: string;
    weakness_type_accepted?: string;
    verdict?: string;
    response?: string;
    concession?: string | null;
  }>;
  strengthened_arguments?: Array<{
    claim_ref?: string;
    r2_challenge_summary?: string;
    additional_evidence?: string | null;
    final_strength?: number;
  }>;
  data_gaps?: string[];
}

type DebateJson = R1DebateJson & R2DebateJson & R3DebateJson;

/* ─── 解析工具 ─── */

/** 从可能的 AgentResult 包装中提取 content */
function unwrapAgentResult(raw: string): string {
  const trimmed = raw.trim();
  if (!trimmed.startsWith("{")) { return raw; }
  try {
    const obj = JSON.parse(trimmed) as Record<string, unknown>;
    if (
      typeof obj.content === "string"
      && (obj.node_id || obj.role || obj.model)
    ) {
      // 这是 AgentResult 包装，返回 content（即使为空）
      return obj.content;
    }
  } catch { /* not JSON */ }
  return raw;
}

/** 尝试解析辩论 JSON */
function tryParseDebate(text: string): DebateJson | null {
  if (!text) { return null; }
  const beautified = tryBeautifyJson(text);
  try {
    return JSON.parse(beautified) as DebateJson;
  } catch {
    /* try below */
  }
  // 手动找第一个 { 到最后一个 }
  const fb = text.indexOf("{");
  const lb = text.lastIndexOf("}");
  if (fb !== -1 && lb !== -1 && lb > fb) {
    try {
      return JSON.parse(text.slice(fb, lb + 1)) as DebateJson;
    } catch {
      /* ignore */
    }
  }
  // 尝试找第一个 [ 到最后一个 ]
  const lb2 = text.indexOf("[");
  const rb2 = text.lastIndexOf("]");
  if (lb2 !== -1 && rb2 !== -1 && rb2 > lb2) {
    try {
      return JSON.parse(text.slice(lb2, rb2 + 1)) as DebateJson;
    } catch {
      /* ignore */
    }
  }
  return null;
}

/** 从无法解析的辩论文本中提取可读内容 */
function extractReadableDebateText(text: string): string {
  if (!text) { return ""; }
  // 1. 尝试找 JSON 字符串值
  const values: string[] = [];
  const strRe = /"([^"\\\n]{3,})"/g;
  let m: RegExpExecArray | null;
  while ((m = strRe.exec(text)) !== null) {
    const v = m[1].trim();
    // 过滤掉常见 JSON 键名和无关内容
    if (
      v.length > 10
      && !/^(core_arguments|resonance_points|preempted_counter_attacks|cross_examination|summary_for_convergence|bull_strength_score|bear_strength_score|data_gaps|claim|point|our_claim|bear_attack|our_response|target_claim_ref|weakness_type|questions|if_unanswered_impact|dimensions|evidence_refs|category|strength|timeliness|weight)$/
        .test(v)
    ) {
      values.push(v);
    }
  }
  if (values.length > 0) {
    return values.join("\n\n");
  }
  // 2. 清理工具调用标签和多余空白
  const cleaned = text
    .replace(/```json\s*[\s\S]*?\s*```/g, "")
    .replace(/\{\s*["']?[a-zA-Z_]+["']?\s*:/g, "")
    .replace(/["']?[a-zA-Z_]+["']?\s*:\s*["']?/g, "")
    .replace(/[{}[\]",]/g, " ")
    .replace(/\{\{[^}]+\}\}/g, "[未解析变量]")
    .replace(/\s+/g, " ")
    .trim();
  return cleaned.length > 20 ? cleaned : text;
}

/** 统一处理辩论输入 */
function processDebateInput(raw: string): DebateContent {
  const cleaned = cleanToolCallTags(raw);
  const unwrapped = unwrapAgentResult(cleaned);
  if (!unwrapped.trim()) {
    return { text: "", parsed: null, empty: true };
  }
  const parsed = tryParseDebate(unwrapped);
  const hasContent = !!parsed && (
    (Array.isArray(parsed.core_arguments) && parsed.core_arguments.length > 0)
    || (Array.isArray(parsed.cross_examination) && parsed.cross_examination.length > 0)
    || (Array.isArray(parsed.resonance_points) && parsed.resonance_points.length > 0)
    || (Array.isArray(parsed.preempted_counter_attacks) && parsed.preempted_counter_attacks.length > 0)
    || (typeof parsed.summary_for_convergence === "string" && parsed.summary_for_convergence.length > 10)
    // R3 最终反驳字段：r2_cross_examination_response / strengthened_arguments 数组任一非空,
    // 或 final_position / claim 字符串非空,均视为有内容。
    || (Array.isArray(parsed.r2_cross_examination_response) && parsed.r2_cross_examination_response.length > 0)
    || (Array.isArray(parsed.strengthened_arguments) && parsed.strengthened_arguments.length > 0)
    || (typeof parsed.final_position === "string" && parsed.final_position.length > 0)
    || (typeof parsed.claim === "string" && parsed.claim.length > 0)
  );
  return {
    text: unwrapped,
    parsed: hasContent ? parsed : null,
    empty: !hasContent && unwrapped.trim().length < 20,
  };
}

/* ─── 渲染组件 ─── */

function R1View({ data, isDark }: { data: R1DebateJson; isDark: boolean }) {
  return (
    <div className="space-y-3">
      {data.core_arguments && data.core_arguments.length > 0 && (
        <div>
          <div className="text-xs font-semibold mb-1" style={{ color: "var(--muted)" }}>核心论点</div>
          <ul className="list-disc pl-4 space-y-1">
            {data.core_arguments.map((arg, i) => (
              <li key={i} className="text-xs">
                <span className="font-medium">{arg.claim || "未命名论点"}</span>
                {arg.category && <Tag className="ml-1 text-xs">{arg.category}</Tag>}
                {typeof arg.strength === "number" && (
                  <span className="ml-1" style={{ color: "var(--sa-red)" }}>强度:{arg.strength}</span>
                )}
                {arg.evidence_refs && arg.evidence_refs.length > 0 && (
                  <div className="text-xs mt-0.5" style={{ color: "var(--muted)" }}>
                    引用: {arg.evidence_refs.slice(0, 2).join("；")}
                  </div>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}

      {data.resonance_points && data.resonance_points.length > 0 && (
        <div>
          <div className="text-xs font-semibold mb-1" style={{ color: "var(--muted)" }}>多维度共振</div>
          <ul className="list-disc pl-4 space-y-1">
            {data.resonance_points.map((rp, i) => (
              <li key={i} className="text-xs">
                {rp.point || "共振点"}
                {rp.dimensions && rp.dimensions.length > 0 && (
                  <span className="ml-1" style={{ color: "var(--muted)" }}>
                    ({rp.dimensions.join("+")})
                  </span>
                )}
                {typeof rp.weight === "number" && (
                  <span className="ml-1" style={{ color: "var(--sa-amber)" }}>权重:{rp.weight}</span>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}

      {data.preempted_counter_attacks && data.preempted_counter_attacks.length > 0 && (
        <div>
          <div className="text-xs font-semibold mb-1" style={{ color: "var(--muted)" }}>反驳预防</div>
          <div className="space-y-2">
            {data.preempted_counter_attacks.map((pa, i) => (
              <div
                key={i}
                className="p-2 rounded text-xs"
                style={{
                  background: isDark ? "rgba(255,255,255,0.04)" : "rgba(0,0,0,0.03)",
                  borderLeft: "2px solid var(--sa-amber)",
                }}
              >
                <div className="font-medium mb-0.5">{pa.our_claim || `论点 ${i + 1}`}</div>
                {pa.bear_attack && (
                  <div className="mb-0.5" style={{ color: "var(--sa-green)" }}>
                    <span className="font-medium">空方攻击:</span> {pa.bear_attack}
                  </div>
                )}
                {pa.our_response && (
                  <div style={{ color: "var(--sa-red)" }}>
                    <span className="font-medium">我方回应:</span> {pa.our_response}
                  </div>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

      {typeof data.bull_strength_score === "number" && (
        <div className="text-xs">
          <Tag color="red">看多强度: {data.bull_strength_score}</Tag>
        </div>
      )}
      {typeof data.bear_strength_score === "number" && (
        <div className="text-xs">
          <Tag color="green">看空强度: {data.bear_strength_score}</Tag>
        </div>
      )}

      {data.data_gaps && data.data_gaps.length > 0 && (
        <div className="text-xs" style={{ color: "var(--muted)" }}>
          数据缺口: {data.data_gaps.join("；")}
        </div>
      )}
    </div>
  );
}

function R2View({ data, isDark }: { data: R2DebateJson; isDark: boolean }) {
  return (
    <div className="space-y-3">
      {data.cross_examination && data.cross_examination.length > 0 && (
        <div>
          <div className="text-xs font-semibold mb-1" style={{ color: "var(--muted)" }}>质询要点</div>
          <div className="space-y-2">
            {data.cross_examination.map((ce, i) => (
              <div
                key={i}
                className="p-2 rounded text-xs"
                style={{
                  background: isDark ? "rgba(255,255,255,0.04)" : "rgba(0,0,0,0.03)",
                  borderLeft: "2px solid var(--primary)",
                }}
              >
                <div className="font-medium mb-0.5">{ce.target_claim_ref || `质询 ${i + 1}`}</div>
                {ce.weakness_type && <Tag className="mb-1 text-xs">{ce.weakness_type}</Tag>}
                {ce.questions && ce.questions.length > 0 && (
                  <ul className="list-decimal pl-4 space-y-0.5">
                    {ce.questions.map((q, j) => <li key={j}>{q}</li>)}
                  </ul>
                )}
                {ce.if_unanswered_impact && (
                  <div className="mt-1" style={{ color: "var(--muted)" }}>
                    <span className="font-medium">影响:</span> {ce.if_unanswered_impact}
                  </div>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

      {data.summary_for_convergence && (
        <div
          className="p-2 rounded text-xs"
          style={{ background: isDark ? "rgba(255,255,255,0.04)" : "rgba(0,0,0,0.03)" }}
        >
          <span className="font-medium">收敛总结:</span> {data.summary_for_convergence}
        </div>
      )}
    </div>
  );
}

/** R3 最终立场标签映射 */
const R3_POSITION_LABEL: Record<string, { text: string; color: string }> = {
  strong_bull: { text: "强烈看多", color: "red" },
  bull: { text: "看多", color: "red" },
  weak_bull: { text: "弱看多", color: "orange" },
  strong_bear: { text: "强烈看空", color: "green" },
  bear: { text: "看空", color: "green" },
  weak_bear: { text: "弱看空", color: "lime" },
};

/** R3 verdict 标签映射 */
const R3_VERDICT_LABEL: Record<string, { text: string; color: string }> = {
  rejected: { text: "反驳", color: "red" },
  partially_accepted: { text: "部分接受", color: "orange" },
  accepted: { text: "接受", color: "green" },
};

function R3View({ data, isDark }: { data: R3DebateJson; isDark: boolean }) {
  const posInfo = data.final_position ? R3_POSITION_LABEL[data.final_position] : null;
  return (
    <div className="space-y-3">
      {/* 最终立场声明 */}
      {posInfo && (
        <div className="flex items-center gap-2 flex-wrap">
          <Tag color={posInfo.color}>{posInfo.text}</Tag>
          {typeof data.confidence === "number" && (
            <span className="text-xs" style={{ color: "var(--muted)" }}>
              置信度 {data.confidence}
            </span>
          )}
        </div>
      )}

      {data.claim && (
        <div
          className="p-2 rounded text-xs"
          style={{
            background: isDark ? "rgba(255,255,255,0.04)" : "rgba(0,0,0,0.03)",
            borderLeft: posInfo?.color === "green" || data.final_position?.includes("bear")
              ? "2px solid var(--sa-green)"
              : "2px solid var(--sa-red)",
          }}
        >
          <span className="font-medium">最终立场:</span> {data.claim}
        </div>
      )}

      {/* R2 质询逐条回应 */}
      {data.r2_cross_examination_response && data.r2_cross_examination_response.length > 0 && (
        <div>
          <div className="text-xs font-semibold mb-1" style={{ color: "var(--muted)" }}>逐条回应 R2 质询</div>
          <div className="space-y-2">
            {data.r2_cross_examination_response.map((resp, i) => {
              const vInfo = resp.verdict ? R3_VERDICT_LABEL[resp.verdict] : null;
              return (
                <div
                  key={i}
                  className="p-2 rounded text-xs"
                  style={{
                    background: isDark ? "rgba(255,255,255,0.04)" : "rgba(0,0,0,0.03)",
                    borderLeft: "2px solid var(--primary)",
                  }}
                >
                  <div className="flex items-center gap-1 flex-wrap mb-0.5">
                    <span className="font-medium">质询 {i + 1}</span>
                    {resp.verdict && (
                      <Tag className="text-xs" color={vInfo?.color ?? "default"}>
                        {vInfo?.text ?? resp.verdict}
                      </Tag>
                    )}
                    {resp.weakness_type_accepted && <Tag className="text-xs">{resp.weakness_type_accepted}</Tag>}
                  </div>
                  {resp.r2_question_ref && (
                    <div className="text-xs mb-0.5" style={{ color: "var(--muted)" }}>
                      针对: {resp.r2_question_ref}
                    </div>
                  )}
                  {resp.response && <div className="text-xs">{resp.response}</div>}
                  {resp.concession && (
                    <div className="text-xs mt-0.5" style={{ color: "var(--sa-amber)" }}>
                      <span className="font-medium">修正:</span> {resp.concession}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}

      {/* 强化保留论据 */}
      {data.strengthened_arguments && data.strengthened_arguments.length > 0 && (
        <div>
          <div className="text-xs font-semibold mb-1" style={{ color: "var(--muted)" }}>强化保留论据</div>
          <ul className="list-disc pl-4 space-y-1">
            {data.strengthened_arguments.map((sa, i) => (
              <li key={i} className="text-xs">
                <span className="font-medium">{sa.claim_ref || `论据 ${i + 1}`}</span>
                {sa.r2_challenge_summary && (
                  <span className="ml-1" style={{ color: "var(--muted)" }}>
                    (R2: {sa.r2_challenge_summary})
                  </span>
                )}
                {typeof sa.final_strength === "number" && (
                  <span className="ml-1" style={{ color: "var(--sa-red)" }}>最终强度:{sa.final_strength}</span>
                )}
                {sa.additional_evidence && (
                  <div className="text-xs mt-0.5" style={{ color: "var(--muted)" }}>
                    补充证据: {sa.additional_evidence}
                  </div>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}

      {/* 数据缺口 */}
      {data.data_gaps && data.data_gaps.length > 0 && (
        <div className="text-xs" style={{ color: "var(--muted)" }}>
          仍未解决的数据缺口: {data.data_gaps.join("；")}
        </div>
      )}
    </div>
  );
}

function DebateContentView({ content, isDark }: { content: DebateContent; isDark: boolean }) {
  if (content.empty) {
    return <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="暂无数据" />;
  }

  // 结构化数据优先
  if (content.parsed) {
    const isR2 = Array.isArray(content.parsed.cross_examination) && content.parsed.cross_examination.length > 0;
    // R3 优先识别:r2_cross_examination_response 或 strengthened_arguments 或 final_position 任一存在
    // 视为 R3 节点(避免 R3 的 r2_cross_examination_response 字段被误判为 R2 的 cross_examination)
    const isR3 = !isR2 && (
      Array.isArray(content.parsed.r2_cross_examination_response)
      || Array.isArray(content.parsed.strengthened_arguments)
      || typeof content.parsed.final_position === "string"
    );
    const isR1 = !isR2 && !isR3 && (
      Array.isArray(content.parsed.core_arguments)
      || Array.isArray(content.parsed.preempted_counter_attacks)
      || Array.isArray(content.parsed.resonance_points)
    );

    if (isR1) {
      return <R1View data={content.parsed} isDark={isDark} />;
    }
    if (isR2) {
      return <R2View data={content.parsed} isDark={isDark} />;
    }
    if (isR3) {
      return <R3View data={content.parsed} isDark={isDark} />;
    }
  }

  // 回退：提取可读文本，禁止显示原始 JSON
  const readable = extractReadableDebateText(content.text);
  if (readable && readable.length > 10) {
    return (
      <div className="text-xs space-y-1" style={{ color: "var(--text-primary)" }}>
        {readable.split(/\n{2,}/).map((para, i) => <p key={i} className="leading-relaxed">{para.trim()}</p>)}
      </div>
    );
  }
  return <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="暂无数据" />;
}

/* ─── 主组件 ─── */

export function DebatePanel() {
  const { t } = useTranslation();
  const themeMode = useSettingsStore((s) => s.settings.theme_mode);
  const isDark = themeMode === "dark"
    || (themeMode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
  const debateRounds = useStockAnalysisStore((s) => s.debateRounds);
  // 阶段 6: 重跑辩论:在 early-return 之前声明 hook,保持 hooks 调用顺序一致
  const startAnalysis = useStockAnalysisStore((s) => s.startAnalysis);
  const [expanded, setExpanded] = useState(false);

  const [sentimentRatio, bullCount, bearCount] = useMemo(() => {
    if (debateRounds.length === 0) { return [0.5, 0, 0]; }
    let bullish = 0;
    let bearish = 0;
    for (const r of debateRounds) {
      if (classifySentiment(r.bull) === "bullish") { bullish++; }
      if (classifySentiment(r.bear) === "bearish") { bearish++; }
    }
    const total = bullish + bearish;
    return [
      total > 0 ? bullish / total : 0.5,
      bullish,
      bearish,
    ];
  }, [debateRounds]);

  const sentimentText = sentimentRatio > 0.55
    ? t("stockAnalysis.sentimentBullish")
    : sentimentRatio < 0.45
    ? t("stockAnalysis.sentimentBearish")
    : t("stockAnalysis.sentimentNeutral");
  const pct = Math.round(sentimentRatio * 100);
  const pointerLeft = `${sentimentRatio * 100}%`;

  // 预处理所有轮次数据
  const processedRounds = debateRounds.map((r) => ({
    round: r.round,
    bull: processDebateInput(r.bull),
    bear: processDebateInput(r.bear),
  }));

  // 阶段 7: 检测失败轮次 + 降级轮次
  // 期望 3 轮辩论,缺失的 round = 失败(LLM 输出空/超时导致整链雪崩,被 stage 1 过滤掉)
  // 降级轮次 = 内容含 [DEGRADED] 前缀(R1/R2 缺失时 LLM 走降级策略)
  const EXPECTED_ROUNDS = [1, 2, 3] as const;
  const presentRounds = new Set(processedRounds.map((r) => r.round));
  const failedRounds = EXPECTED_ROUNDS.filter((r) => !presentRounds.has(r));
  const degradedRounds = processedRounds.filter(
    (r) => r.bull.text.includes("[DEGRADED]") || r.bear.text.includes("[DEGRADED]"),
  );
  const showWarning = failedRounds.length > 0 || degradedRounds.length > 0;

  // P1.5-5: Round 切换状态
  const [activeRoundIdx, setActiveRoundIdx] = useState(0);
  const activeRound = processedRounds[activeRoundIdx];

  // 找 R3 裁决卡片: 从所有轮次中提取最终立场
  // processedRounds 在本组件内创建且不突变，使用 useMemo 的唯一目的是传稳定引用
  const finalVerdict = useMemo(() => {
    for (const r of processedRounds) {
      if (r.bull.parsed?.final_position || r.bull.parsed?.claim) {
        return r.bull.parsed;
      }
      if (r.bear.parsed?.final_position || r.bear.parsed?.claim) {
        return r.bear.parsed;
      }
    }
    return null;
  // eslint-disable-next-line react-hooks/preserve-manual-memoization
  }, [processedRounds]);

  if (debateRounds.length === 0) { return null; }

  // 阶段 6: 重跑辩论 — 直接触发一次 stock-analysis workflow
  // (用 stockAnalysisStore 的 startAnalysis;不引入新后端命令)
  // startAnalysis hook 已在组件顶部声明
  const handleRerun = () => {
    const { stockCode } = useStockAnalysisStore.getState();
    if (!stockCode) { return; }
    void startAnalysis(stockCode);
  };

  // Round 标签
  const roundOptions = processedRounds.map((r, i) => ({
    label: `第${r.round}轮`,
    value: i,
  }));

  return (
    <>
      <Card
        size="small"
        title={
          <div className="flex items-center gap-2">
            <span>{t("stockAnalysis.debate")}</span>
            {finalVerdict && (finalVerdict.final_position || finalVerdict.claim) && (
              <Tag color={finalVerdict.final_position?.includes("bear") ? "green" : "red"}>
                最终裁决: {finalVerdict.claim?.slice(0, 30) || finalVerdict.final_position || "待定"}
              </Tag>
            )}
          </div>
        }
        extra={
          <div className="flex items-center gap-1">
            {showWarning && (
              <Button
                type="text"
                size="small"
                icon={<ReloadOutlined />}
                onClick={handleRerun}
                title={t("stockAnalysis.reflection.rerunDebate")}
              />
            )}
            <Button
              type="text"
              size="small"
              icon={<ExpandOutlined />}
              onClick={() => setExpanded(true)}
            />
          </div>
        }
      >
        {showWarning && (
          <Alert
            type="warning"
            showIcon
            icon={<WarningOutlined />}
            className="mb-2"
            message={failedRounds.length > 0
              ? t("stockAnalysis.debateRoundsFailed", { rounds: failedRounds.join(", ") })
              : t("stockAnalysis.debateRoundsDegraded", { rounds: degradedRounds.map((r) => r.round).join(", ") })}
            description={failedRounds.length > 0
              ? t("stockAnalysis.debateRoundsFailedDesc")
              : t("stockAnalysis.debateRoundsDegradedDesc")}
          />
        )}
        <div className="mb-3">
          <div className="flex justify-between text-xs mb-1">
            <span style={{ color: "var(--sa-green)" }}>{t("stockAnalysis.bear")}</span>
            <span className="font-semibold" style={{ fontSize: 13 }}>{sentimentText}</span>
            <span style={{ color: "var(--sa-red)" }}>{t("stockAnalysis.bull")}</span>
          </div>
          <div
            className="relative"
            style={{
              height: 18,
              borderRadius: 9,
              background: "linear-gradient(to right, var(--sa-green), var(--sa-amber), var(--sa-red))",
              overflow: "hidden",
            }}
          >
            <div
              style={{
                position: "absolute",
                left: pointerLeft,
                top: 0,
                width: 2,
                height: 18,
                background: "var(--surface)",
                boxShadow: "0 0 4px rgba(0,0,0,0.4)",
                transform: "translateX(-1px)",
                zIndex: 2,
              }}
            />
          </div>
          <div className="flex justify-between text-xs mt-1">
            <span style={{ color: "var(--muted)" }}>
              🐻 {bearCount}
              {t("stockAnalysis.views")}
            </span>
            <span className="font-mono" style={{ color: "var(--muted)" }}>
              {pct}:{100 - pct}
            </span>
            <span style={{ color: "var(--muted)" }}>
              🐂 {bullCount}
              {t("stockAnalysis.views")}
            </span>
          </div>
        </div>

        {/* P1.5-5: Round 标签切换代替原有的 Collapse */}
        {processedRounds.length > 0 && (
          <div className="mb-2">
            <Segmented
              size="small"
              value={activeRoundIdx}
              options={roundOptions}
              onChange={(val) => setActiveRoundIdx(val as number)}
              className="mb-2"
            />
            {/* 裁决卡片高亮 */}
            {finalVerdict && roundOptions[activeRoundIdx]?.label?.includes("3") && (
              <div
                className="p-2 rounded text-xs mb-2 font-medium"
                style={{
                  background: "linear-gradient(135deg, rgba(124,58,237,0.12), rgba(124,58,237,0.04))",
                  border: "1px solid rgba(124,58,237,0.3)",
                  borderRadius: 8,
                }}
              >
                <div className="flex items-center gap-2 mb-1">
                  <span role="img" aria-label="gavel">⚖️</span>
                  <span className="font-semibold" style={{ color: "#7c3aed" }}>裁决理由</span>
                </div>
                <Typography.Paragraph
                  ellipsis={{ rows: 3, expandable: true, symbol: "展开" }}
                  className="mb-0"
                  style={{ fontSize: 11, color: "var(--color-text-secondary)" }}
                >
                  {finalVerdict.claim || "暂无裁决说明"}
                </Typography.Paragraph>
              </div>
            )}
            {/* 当前轮次的辩论内容 */}
            {activeRound && (
              <div className="flex flex-col sm:flex-row gap-2" style={{ maxHeight: 360, overflow: "auto" }}>
                <div className="flex-1 p-2 rounded" style={{ borderLeft: "3px solid var(--sa-red)" }}>
                  <Tag color="red">{t("stockAnalysis.bull")}</Tag>
                  <div className="mt-1">
                    <DebateContentView content={activeRound.bull} isDark={isDark} />
                  </div>
                </div>
                <div className="flex-1 p-2 rounded" style={{ borderLeft: "3px solid var(--sa-green)" }}>
                  <Tag color="green">{t("stockAnalysis.bear")}</Tag>
                  <div className="mt-1">
                    <DebateContentView content={activeRound.bear} isDark={isDark} />
                  </div>
                </div>
              </div>
            )}
          </div>
        )}
      </Card>

      <Modal
        title={t("stockAnalysis.debate")}
        open={expanded}
        onCancel={() => setExpanded(false)}
        footer={null}
        width="85vw"
        style={{ top: 20 }}
        styles={{ body: { maxHeight: "80vh", overflow: "auto" } }}
      >
        <div className="mb-3">
          <div className="flex justify-between text-sm mb-1">
            <span style={{ color: "var(--sa-green)" }}>{t("stockAnalysis.bear")}</span>
            <span className="font-semibold">{sentimentText}</span>
            <span style={{ color: "var(--sa-red)" }}>{t("stockAnalysis.bull")}</span>
          </div>
          <div
            className="relative"
            style={{
              height: 22,
              borderRadius: 11,
              background: "linear-gradient(to right, var(--sa-green), var(--sa-amber), var(--sa-red))",
              overflow: "hidden",
            }}
          >
            <div
              style={{
                position: "absolute",
                left: pointerLeft,
                top: 0,
                width: 2,
                height: 22,
                background: "var(--surface)",
                boxShadow: "0 0 4px rgba(0,0,0,0.4)",
                transform: "translateX(-1px)",
                zIndex: 2,
              }}
            />
          </div>
        </div>
        {/* P1.5-5: Modal 内的 Round 切换 */}
        {processedRounds.length > 0 && (
          <div className="mb-3">
            <Segmented
              size="small"
              value={activeRoundIdx}
              options={roundOptions}
              onChange={(val) => setActiveRoundIdx(val as number)}
              className="mb-3"
            />
            {/* 裁决卡片高亮（全屏模式） */}
            {finalVerdict && roundOptions[activeRoundIdx]?.label?.includes("3") && (
              <div
                className="p-3 rounded mb-3"
                style={{
                  background: "linear-gradient(135deg, rgba(124,58,237,0.12), rgba(124,58,237,0.04))",
                  border: "1px solid rgba(124,58,237,0.3)",
                  borderRadius: 8,
                }}
              >
                <div className="flex items-center gap-2 mb-2">
                  <span style={{ fontSize: 20 }} role="img" aria-label="gavel">⚖️</span>
                  <span className="font-semibold" style={{ color: "#7c3aed" }}>最终裁决</span>
                </div>
                <Typography.Paragraph
                  ellipsis={{ rows: 5, expandable: true, symbol: "展开全文" }}
                  className="mb-1"
                  style={{ fontSize: 12, color: "var(--color-text-secondary)" }}
                >
                  {finalVerdict.claim || "暂无裁决说明"}
                </Typography.Paragraph>
                {finalVerdict.final_position && (
                  <Tag color={finalVerdict.final_position.includes("bear") ? "green" : "red"}>
                    立场: {finalVerdict.final_position}
                  </Tag>
                )}
                {typeof finalVerdict.confidence === "number" && (
                  <Tag color="blue">
                    置信度: {finalVerdict.confidence}
                  </Tag>
                )}
              </div>
            )}
            {/* 当前轮次 */}
            {activeRound && (
              <div className="flex flex-col sm:flex-row gap-3">
                <div className="flex-1 p-3 rounded" style={{ borderLeft: "4px solid var(--sa-red)" }}>
                  <Tag color="red">{t("stockAnalysis.bull")}</Tag>
                  <div className="mt-2">
                    <DebateContentView content={activeRound.bull} isDark={isDark} />
                  </div>
                </div>
                <div className="flex-1 p-3 rounded" style={{ borderLeft: "4px solid var(--sa-green)" }}>
                  <Tag color="green">{t("stockAnalysis.bear")}</Tag>
                  <div className="mt-2">
                    <DebateContentView content={activeRound.bear} isDark={isDark} />
                  </div>
                </div>
              </div>
            )}
          </div>
        )}
      </Modal>
    </>
  );
}
