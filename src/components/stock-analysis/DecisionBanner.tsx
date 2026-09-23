import { extractLlmField } from "@/lib/agentOutput";
import { reportQualitySeverity } from "@/lib/dataQualityDiagnosis";
import { type DecisionInputsReport, summarizeDecisionInputs } from "@/lib/decisionInputDiagnosis";
import { showBackendError } from "@/lib/errorI18n";
import { invoke } from "@/lib/invoke";
import { exportAnalysisReport } from "@/lib/stock-analysis-export";
import { type ExportData, type ExportFormat } from "@/lib/stock-analysis-export";
import {
  actionToDirection,
  agreementBgColor,
  agreementColor,
  computeStockConsensus,
  getActionColor,
  getActionTKey,
  getRiskColor,
  getRiskTKey,
  resolveDisplayAction,
} from "@/lib/stock-analysis-utils";
import { useSettingsStore, useStockAnalysisStore } from "@/stores";
import { useTimeAnchorStore } from "@/stores/feature/timeAnchorStore";
import type { DataQualityReport, HorizonPriceGroup } from "@/types";
import { ExpandOutlined, FilePptOutlined, FileTextOutlined, FileWordOutlined, ReloadOutlined } from "@ant-design/icons";
import { App, Button, Card, Collapse, Dropdown, Modal, Tag, Tooltip } from "antd";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router-dom";
import { DecisionExplanationPanel } from "./DecisionExplanationPanel";
import { DecisionTrustNotice } from "./DecisionTrustNotice";
import { ReportMarkdown } from "./ReportMarkdown";
import { cleanToolCallTags } from "./utils";

export function DecisionBanner({ embeddedInWorkspace = false }: { embeddedInWorkspace?: boolean }) {
  const { message } = App.useApp();
  const { t } = useTranslation();
  const navigate = useNavigate();
  const themeMode = useSettingsStore((s) => s.settings.themeMode);
  const isDark = themeMode === "dark"
    || (themeMode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
  const decision = useStockAnalysisStore((s) => s.decision);
  // 2026-09-22: 展示档 = 方向档（`resolveDisplayAction` 已恒等，不再按仓位派生）——
  // 「持有 / 观望」不再由组件各自判断仓位，也不再由**本次建议仓位**反推（那是循环判据，
  // 见 `AUDIT-300642-run-variance-2026-09-22.md`）。后端 action 已只表达方向强度。
  // ⚠️ 只用于**展示**（Tag 文案 + 配色）。方向判定（actionToDirection）与
  //    「问 AI」prompt 仍读 decision.action 原文，避免派生把原始结论改写掉。
  const displayAction = useMemo(
    () =>
      decision
        ? resolveDisplayAction(decision.action, decision.positionState, decision.positionPct)
        : null,
    [decision],
  );
  const stockCode = useStockAnalysisStore((s) => s.stockCode);
  const stockName = useStockAnalysisStore((s) => s.stockName);
  // 重跑分析: 透传当前 analysisId 让后端"覆盖"同 id 旧记录(而非新建一条)
  const analysisId = useStockAnalysisStore((s) => s.analysisId);
  // 加载失败错误(loadAnalysis reject 时由 StockAnalysisPage 写入)
  const error = useStockAnalysisStore((s) => s.error);
  const quote = useStockAnalysisStore((s) => s.quote);
  const analystReports = useStockAnalysisStore((s) => s.analystReports);
  const debateRounds = useStockAnalysisStore((s) => s.debateRounds);
  const riskAssessments = useStockAnalysisStore((s) => s.riskAssessments);
  const valueAssessments = useStockAnalysisStore((s) => s.valueAssessments);
  const dataQualitySummary = useStockAnalysisStore((s) => s.dataQualitySummary) ?? "";
  // 决策输入诊断：portfolio-mgr 16 个上游节点的数据符合度（纯前端，不持久化）
  const decisionInputsReport = useStockAnalysisStore((s) => s.decisionInputsReport) ?? [];
  const failedNodes = useStockAnalysisStore((s) => s.failedNodes);
  const dataWarnings = useStockAnalysisStore((s) => s.dataWarnings);
  const ruleCheckResults = useStockAnalysisStore((s) => s.ruleCheckResults);
  const rawData = useStockAnalysisStore((s) => s.rawData);
  const bumpWatchlistVersion = useStockAnalysisStore((s) => s.bumpWatchlistVersion);
  const watchlistVersion = useStockAnalysisStore((s) => s.watchlistVersion);
  // P0-1: 证据质量驱动共识
  const stockCodeEvidence = useStockAnalysisStore((s) => stockCode ? s.evidenceReport?.[stockCode] : null);
  // 方案 D 双向并存: LLM 决策 JSON + 一致性分数
  const llmDecisionJson = useStockAnalysisStore((s) => s.llmDecisionJson);
  const decisionAgreementScore = useStockAnalysisStore((s) => s.decisionAgreementScore);
  // 分歧阈值（从工作流模板读取,默认 40）
  const [disagreementThreshold, setDisagreementThreshold] = useState(40);
  useEffect(() => {
    invoke<Record<string, unknown>>("get_workflow_template", { id: "stock-analysis" })
      .then((tmpl) => {
        const vars = (tmpl?.variables ?? []) as Record<string, unknown>[];
        const v = vars.find((x: Record<string, unknown>) => x.name === "dual_view_disagreement_threshold");
        if (v && typeof v.value === "number") {
          setDisagreementThreshold(v.value);
        }
      })
      .catch(() => {});
  }, []);
  // 解析 LLM stance + summary 用于 banner 展示
  const llmStance = useMemo(() => {
    if (!llmDecisionJson) { return null; }
    return (extractLlmField(llmDecisionJson, "action") as string | null)
      ?? (extractLlmField(llmDecisionJson, "stance") as string | null)
      ?? null;
  }, [llmDecisionJson]);
  const llmSummary = useMemo(() => {
    if (!llmDecisionJson) { return null; }
    return (extractLlmField(llmDecisionJson, "reasoning") as string | null)
      ?? (extractLlmField(llmDecisionJson, "summary") as string | null)
      ?? null;
  }, [llmDecisionJson]);
  // V65: 解析 LLM 完整字段（仓位/风险/缺口/证据）用于双视角对比
  const llmFields = useMemo(() => {
    if (!llmDecisionJson) {
      return null;
    }
    const pos = extractLlmField(llmDecisionJson, "positionPct") as number | null;
    const conf = extractLlmField(llmDecisionJson, "confidence") as number | null;
    const risk = extractLlmField(llmDecisionJson, "riskLevel") as string | null;
    const gaps = extractLlmField(llmDecisionJson, "data_gaps") as string[] | null;
    const evidence = extractLlmField(llmDecisionJson, "evidence_cited") as
      | Array<{ source?: string; point?: string }>
      | null;
    const targetPrice = extractLlmField(llmDecisionJson, "targetPrice") as number | null;
    const stopLoss = extractLlmField(llmDecisionJson, "stopLoss") as number | null;
    const stopLossPct = extractLlmField(llmDecisionJson, "stopLossPct") as number | null;
    const takeProfitPct = extractLlmField(llmDecisionJson, "takeProfitPct") as number | null;
    return {
      pos,
      conf,
      risk,
      gaps: Array.isArray(gaps) ? gaps : null,
      evidence: Array.isArray(evidence) ? evidence : null,
      targetPrice,
      stopLoss,
      stopLossPct,
      takeProfitPct,
    };
  }, [llmDecisionJson]);
  // timeline-jump 高亮：被 evidence 指向时短暂加 ring 样式
  const highlightedPanel = useStockAnalysisStore((s) => s.highlightedPanel);
  // 时间旅行: 当前决策所基于的 as-of 锚点 (live 时为 null)
  const asOfDate = useTimeAnchorStore((s) => s.asOfDate);
  const [adding, setAdding] = useState(false);
  const [watchlisted, setWatchlisted] = useState(false);
  const [expanded, setExpanded] = useState(false);
  // V50+: 分歧时内联展开双视角对比（替代跳 tab）
  const [showInlineComparison, setShowInlineComparison] = useState(false);
  // 快速交易录入 ref（替代 getElementById，防止多实例 ID 冲突）
  const tradePriceRef = useRef<HTMLInputElement>(null);
  const tradeQtyRef = useRef<HTMLInputElement>(null);

  // stockCode 或自选列表变化时同步自选状态
  useEffect(() => {
    if (!stockCode) { return; }
    let cancelled = false;
    (async () => {
      try {
        const list = await invoke<{ stockCode: string }[]>("list_watchlist");
        if (!cancelled) {
          setWatchlisted(list.some((w) => w.stockCode === stockCode));
        }
      } catch {
        if (!cancelled) { setWatchlisted(false); }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [stockCode, watchlistVersion]);

  const addToWatchlist = useCallback(async () => {
    if (!stockCode || !stockName) { return; }
    setAdding(true);
    try {
      await invoke("add_to_watchlist", { stockCode, stockName });
      setWatchlisted(true);
      bumpWatchlistVersion();
      message.success(t("stockAnalysis.addedToWatchlist"));
    } catch {
      message.error(t("stockAnalysis.addFailed"));
    }
    setAdding(false);
  }, [stockCode, stockName, t, bumpWatchlistVersion, message]);

  const actionLabel = (action: string) => t(getActionTKey(action));

  // 共享的决策上下文计算（避免 handleExport / handleAskAI 重复计算）
  const decisionContext = useMemo(() => {
    if (!decision || !stockCode || !stockName) { return null; }
    const currentPrice = quote?.price ?? 0;
    const targetPriceNum = decision.targetPrice != null ? Number(decision.targetPrice) : 0;
    const upside = targetPriceNum > 0 && currentPrice > 0
      ? ((targetPriceNum - currentPrice) / currentPrice * 100)
      : null;
    const confidencePct = Math.round(decision.confidence ?? 0);
    return { currentPrice, targetPriceNum, upside, confidencePct };
  }, [decision, stockCode, stockName, quote]);

  // 解析 data-quality 节点输出的 JSON 诊断报告
  // data-quality 是 CodeNode + Rhai，输出为 JSON 字符串（非 agent_executor 包装的 verdict 结构）
  //
  // 兼容两种格式：
  //   1. 新版（2026-07-23+）：含 diagnostics/missing_analysts/low_confidence_analysts/summary
  //   2. 旧版（仅 grade/score/gap_count 等 6 字段）：无 diagnostics，但既然用户能看到 grade F
  //      且 gap_count=N 的"全缺失"场景，仍降级渲染一个简化的诊断面板——
  //      把 gap_count 个分析师标记为 missing，good_count 个标 normal，让用户立即看到
  //      "哪几个分析师实际上有数据" vs "哪几个全没数据"。
  //   3. 解析失败或非 JSON：返回 null，不渲染。
  const dataQualityReport = useMemo<DataQualityReport | null>(() => {
    if (!dataQualitySummary || !dataQualitySummary.trim()) { return null; }
    let parsed: any = null;
    try {
      parsed = JSON.parse(dataQualitySummary);
    } catch {
      return null;
    }
    if (!parsed || typeof parsed !== "object" || typeof parsed.grade !== "string") {
      return null;
    }
    // V66 修复(2026-07-29): stale_record 标记的占位 JSON（snapshot 为空时由
    // stockAnalysisStore.loadAnalysis else 分支设置），返回 null 触发降级面板,
    // 降级面板会检测 stale_record 并显示「重跑分析」按钮。
    if (parsed.stale_record === true) {
      return null;
    }

    // 1. 新版：含 diagnostics 字段
    if (typeof parsed.diagnostics === "object" && parsed.diagnostics !== null) {
      return parsed as DataQualityReport;
    }

    // 2. 旧版降级：构造 10 个分析师的固定列表
    //    前 good_count 个标 normal（按固定顺序），其余标 missing
    //    旧版无法区分 low/normal，统一按 gap_count/missing 重建
    const fallbackOrder: Array<{ abbr: string; nameKey: string; expectedKey: string }> = [
      {
        abbr: "mk",
        nameKey: "stockAnalysis.workflow.analyst.a-market-analyst",
        expectedKey: "stockAnalysis.dataQuality.expectedMarket",
      },
      {
        abbr: "sent",
        nameKey: "stockAnalysis.workflow.analyst.a-sentiment",
        expectedKey: "stockAnalysis.dataQuality.expectedSentiment",
      },
      {
        abbr: "news",
        nameKey: "stockAnalysis.dataQuality.fallbackNewsAnalyst",
        expectedKey: "stockAnalysis.dataQuality.expectedNews",
      },
      {
        abbr: "fund",
        nameKey: "stockAnalysis.workflow.analyst.a-fundamentals",
        expectedKey: "stockAnalysis.dataQuality.expectedFundamentals",
      },
      {
        abbr: "pol",
        nameKey: "stockAnalysis.workflow.analyst.a-policy",
        expectedKey: "stockAnalysis.dataQuality.expectedPolicy",
      },
      {
        abbr: "hm",
        nameKey: "stockAnalysis.workflow.analyst.a-hot-money",
        expectedKey: "stockAnalysis.dataQuality.expectedHotMoney",
      },
      {
        abbr: "lk",
        nameKey: "stockAnalysis.workflow.analyst.a-lockup",
        expectedKey: "stockAnalysis.dataQuality.expectedLockup",
      },
      {
        abbr: "res",
        nameKey: "stockAnalysis.analystRoles.researchAnalyst",
        expectedKey: "stockAnalysis.dataQuality.expectedResearch",
      },
      {
        abbr: "sec",
        nameKey: "stockAnalysis.workflow.analyst.a-sector",
        expectedKey: "stockAnalysis.dataQuality.expectedSector",
      },
      {
        abbr: "cat",
        nameKey: "stockAnalysis.dataQuality.fallbackCatalystAnalyst",
        expectedKey: "stockAnalysis.dataQuality.expectedCatalyst",
      },
    ];
    const totalAnalysts = Number(parsed.total_analysts) || fallbackOrder.length;
    const goodCount = Number(parsed.good_count) || 0;
    const gapCount = Number(parsed.gap_count) || 0;
    const avgConf = Number(parsed.avg_confidence) || 0;

    const diagnostics: Record<string, any> = {};
    const missingList: string[] = [];
    for (let i = 0; i < fallbackOrder.length && i < totalAnalysts; i++) {
      const { abbr, nameKey, expectedKey } = fallbackOrder[i];
      if (i < goodCount) {
        diagnostics[abbr] = {
          name: t(nameKey),
          expected_data: t(expectedKey),
          confidence: avgConf,
          status: avgConf < 50 ? "low" : "normal",
          gap_reason: i < goodCount && gapCount === 0 && avgConf < 50
            ? t("stockAnalysis.dataQuality.gapReasonLegacyConfidence", { avgConf })
            : "",
        };
      } else {
        diagnostics[abbr] = {
          name: t(nameKey),
          expected_data: t(expectedKey),
          confidence: -1,
          status: "missing",
          gap_reason: t("stockAnalysis.dataQuality.gapReasonLegacy"),
        };
        missingList.push(t(nameKey));
      }
    }

    return {
      grade: parsed.grade,
      score: Number(parsed.score) || 0,
      gap_count: gapCount,
      good_count: goodCount,
      avg_confidence: avgConf,
      total_analysts: totalAnalysts,
      diagnostics,
      missing_analysts: missingList,
      low_confidence_analysts: [],
      summary: t("stockAnalysis.dataQuality.legacySummary", {
        grade: parsed.grade,
        score: parsed.score,
        gapCount,
        goodCount,
      }),
    };
  }, [dataQualitySummary, t]);

  // V66 修复(2026-07-29): 检测 stale_record 标记（snapshot 为空时 store 设置的占位 JSON）
  // 用于在降级面板中显示「重跑分析」按钮，而非静默展示空字符串
  const staleRecordSummary: string | null = useMemo(() => {
    if (!dataQualitySummary) { return null; }
    try {
      const parsed = JSON.parse(dataQualitySummary);
      if (parsed?.stale_record === true && typeof parsed.summary === "string") {
        return parsed.summary as string;
      }
    } catch {
      // 忽略解析失败，走默认降级面板
    }
    return null;
  }, [dataQualitySummary]);

  // Hooks 必须在 early return 之前 — 闭包内部自己处理 null decision
  const [exporting, setExporting] = useState<string | null>(null);

  const handleExport = useCallback(async (format: ExportFormat) => {
    if (!decision || !stockCode || !stockName) { return; }
    setExporting(format);
    try {
      const exportData: ExportData = {
        stockCode,
        stockName,
        asOfDate,
        quote: quote
          ? {
            price: quote.price,
            change: quote.price - quote.preClose,
            changePct: quote.changePct,
            high: quote.high,
            low: quote.low,
            volume: quote.volume,
            amount: quote.amount,
          }
          : null,
        analystReports,
        debateRounds,
        riskAssessments,
        valueAssessments,
        decision,
        llmDecisionJson,
        dataQualitySummary,
        ruleCheckResults,
        rawData,
        failedNodes,
        dataWarnings,
      };
      const result = await exportAnalysisReport(exportData, format, t);
      message.success(result);
    } catch (e: unknown) {
      const errMsg = e instanceof Error ? e.message : String(e);
      // 打印完整堆栈 + 数据规模上下文，方便区分 pandoc / 工具未注册 / 路径权限 等失败原因
      console.error("[stock-analysis] export failed:", e, {
        format,
        stockCode,
        stockName,
        asOfDate,
        hasAnalystReports: Object.keys(analystReports).length,
        hasDebate: debateRounds.length,
        hasRisk: Object.keys(riskAssessments).length,
        hasValue: Object.keys(valueAssessments).length,
        hasRule: Object.keys(ruleCheckResults).length,
        failedNodesCount: failedNodes.length,
        dataWarningsCount: dataWarnings?.length ?? 0,
      });
      message.error(t("stockAnalysis.decision.exportFailed", { errMsg }));
    } finally {
      setExporting(null);
    }
  }, [
    decision,
    stockCode,
    stockName,
    asOfDate,
    quote,
    analystReports,
    debateRounds,
    riskAssessments,
    valueAssessments,
    llmDecisionJson,
    dataQualitySummary,
    ruleCheckResults,
    rawData,
    failedNodes,
    dataWarnings,
    t,
    message,
  ]);

  const exportMenuItems: { key: ExportFormat; icon: React.ReactNode; label: string }[] = [
    { key: "md", icon: <FileTextOutlined />, label: "Markdown (.md)" },
    { key: "docx", icon: <FileWordOutlined />, label: t("stockAnalysis.decision.wordDocument") },
    { key: "pptx", icon: <FilePptOutlined />, label: "PowerPoint (.pptx)" },
  ];

  const handleAskAI = useCallback(() => {
    const ctx = decisionContext;
    if (!ctx || !decision || !stockCode || !stockName) { return; }
    const { currentPrice, upside, confidencePct } = ctx;
    const context = [
      t("stockAnalysis.askAi.prompt", { stockName, code: stockCode }),
      // 喂给 AI 的决策档位必须与界面**同一口径**：`decision.action` 是英文枚举，
      // 直接插值会让上下文里写 "HOLD"、界面却显示「观望」，用户拿这个上下文提问时
      // 两边对不上（同 `getRiskTKey` 的处理）。
      decision
        ? t("stockAnalysis.askAi.decision", {
          action: t(
            getActionTKey(
              resolveDisplayAction(decision.action, decision.positionState, decision.positionPct),
            ),
          ),
          confidence: confidencePct,
        })
        : "",
      t("stockAnalysis.export.price", { price: currentPrice.toFixed(2) }),
      upside != null ? t("stockAnalysis.export.upside", { pct: (upside >= 0 ? "+" : "") + upside.toFixed(1) }) : "",
      `${t(getRiskTKey(decision.riskLevel))}`,
    ].filter(Boolean).join("\n");

    navigator.clipboard.writeText(context).then(() => {
      message.success(t("stockAnalysis.contextCopied"));
      navigate(`/chat?code=${stockCode}`);
    }).catch(() => {
      navigate(`/chat?code=${stockCode}`);
    });
  }, [decision, stockCode, stockName, decisionContext, navigate, t, message]);

  // ── 决策缺失占位 ──
  // normalizeDecision 入口已保证非空对象，所以这里的 !decision 意味着：
  //   1) LLM 输出无法解析出 decision（portfolio-mgr 节点结果残缺/被截断）
  //   2) 决策 JSON 是全零空壳（loadAnalysis 已主动跳过 set）
  //   3) loadAnalysis 本身失败（store.error 有值，stockCode 同步预填过但记录详情加载失败）
  // 此时不再 return null，而是渲染"决策缺失"占位卡，让用户知道工作流
  // 已完成但决策信息不完整，并提供"重跑 / 重试"入口。
  if (!decision) {
    // 修复：loadAnalysis 失败时优先渲染"加载失败 + 重试"，避免永远停在
    // "搜索股票"分支(用户认为这违反"history 已有 stockCode"的预期)。
    // StockAnalysisPage 的 useEffect 已从 history 缓存同步预填 stockCode，
    // 所以 error 状态下 stockCode 通常是有值的 → 走"重试"分支。
    if (error) {
      return (
        <Card
          id="decision-banner-top"
          size="small"
          styles={{ body: { padding: "12px 16px" } }}
          style={{
            borderLeft: "4px solid var(--sa-red, #ef4444)",
          }}
          data-testid="decision-banner-load-failed"
        >
          <div className="flex items-start gap-2">
            <span style={{ fontSize: 18, lineHeight: 1 }}>⚠️</span>
            <div className="flex-1">
              <div className="text-sm font-semibold mb-1">
                {t("stockAnalysis.loadAnalysisFailed")}
              </div>
              <div
                className="text-sm mb-2"
                style={{ color: "var(--muted)", wordBreak: "break-all" }}
              >
                {error}
              </div>
              <Button
                size="small"
                type="primary"
                onClick={() => {
                  // 重试：清掉 error 状态后重新调用 loadAnalysis
                  useStockAnalysisStore.setState({ error: null });
                  if (analysisId) {
                    void useStockAnalysisStore.getState().loadAnalysis(analysisId);
                  }
                }}
              >
                {t("stockAnalysis.retry")}
              </Button>
            </div>
          </div>
        </Card>
      );
    }
    return (
      <Card
        id="decision-banner-top"
        size="small"
        styles={{ body: { padding: "12px 16px" } }}
        style={{
          borderLeft: "4px solid var(--sa-amber, #f59e0b)",
          ...(highlightedPanel === "decision"
            ? { boxShadow: "0 0 0 3px var(--accent)", transition: "box-shadow 0.4s" }
            : {}),
        }}
        data-testid="decision-banner-missing"
      >
        <div className="flex items-start gap-2">
          <span style={{ fontSize: 18, lineHeight: 1 }}>⚠️</span>
          <div className="flex-1">
            <div className="text-sm font-semibold mb-1">
              {t("stockAnalysis.decisionMissing")}
            </div>
            <div className="text-sm" style={{ color: "var(--muted)" }}>
              {t("stockAnalysis.decisionMissingHint")}
            </div>
            {stockCode
              ? (
                <div className="mt-2 flex flex-wrap items-center gap-2">
                  {/* 全量重跑：重新执行整个 DAG */}
                  <Button
                    size="small"
                    type="primary"
                    onClick={() =>
                      useStockAnalysisStore.getState().startAnalysis(
                        stockCode,
                        // 重跑覆盖原 analysisId 对应记录(不传则后端新建 UUID)。
                        // 从决策缺失占位卡点出时 store.analysisId 必然是已点开的历史记录 id。
                        analysisId ? { parentAnalysisId: analysisId } : undefined,
                      )}
                  >
                    {t("stockAnalysis.reAnalyze")}
                  </Button>
                  {/* 仅重跑决策：从 blackboard_snapshot 恢复上游输出，只执行 portfolio-mgr。分析师/辩论/风险评估数据已完整时无需全量重跑 */}
                  {analysisId && (
                    <Button
                      size="small"
                      icon={<span>⚡</span>}
                      onClick={() => useStockAnalysisStore.getState().rerunDecision(analysisId)}
                    >
                      {t("stockAnalysis.rerunDecision")}
                    </Button>
                  )}
                </div>
              )
              : (
                <div className="mt-2 flex items-center gap-2">
                  <Button
                    size="small"
                    type="primary"
                    onClick={() => {
                      // 占位阶段 stockCode 还没就绪(极少见:用户从未打开过 history 缓存,
                      // 也未在搜索栏选过股票,直接通过分享链接进入):
                      // 聚焦顶部搜索栏让用户输入。
                      const search = document.querySelector<HTMLInputElement>(
                        "[data-testid='stock-analysis-search-input']",
                      );
                      search?.focus();
                      message.info(t("stockAnalysis.reAnalyzeNeedCode"));
                    }}
                  >
                    {t("stockAnalysis.searchStock")}
                  </Button>
                  <span className="text-sm" style={{ color: "var(--muted)" }}>
                    {t("stockAnalysis.reAnalyzeNeedCodeHint")}
                  </span>
                </div>
              )}
          </div>
        </div>
      </Card>
    );
  }

  // TypeScript 在此之后已将 decision 收窄为 StockDecision (non-null)

  // ── 矛盾检测 ──
  // 1. 后端检测：trader 输出自相矛盾（action 与 targetPrice 方向冲突）
  // 2. 前端检测：决策 action 与分析师共识矛盾
  const isBackendContradictory = !!decision?.isContradictory;
  const isConsensusContradictory = (() => {
    if (!analystReports || Object.keys(analystReports).length < 3 || !decision) { return false; }
    const consensus = computeStockConsensus(analystReports, undefined, decision.timeHorizon);
    // P1-6(2026-09-14): 判定改走统一归一化。原为英文字面量比较，而 portfolio-mgr
    // 产出的是中文「买入/增持/卖出/减持」⇒ 中文值域下两者恒 false，矛盾检测静默失效。
    const consensusDirection = actionToDirection(decision.action);
    const isBullishAction = consensusDirection === "buy";
    const isBearishAction = consensusDirection === "sell";
    if (isBullishAction && (consensus.consensus === "bearish" || consensus.consensus === "divided")) { return true; }
    if (isBearishAction && (consensus.consensus === "bullish" || consensus.consensus === "divided")) { return true; }
    return false;
  })();
  const isContradictory = isBackendContradictory || isConsensusContradictory;

  // V68 修复(2026-07-30): 直接使用公式决策的 confidence，不再被 LLM 决策调制
  //   原问题: adjustedConfidence 会因为公式与 LLM 对立而被惩罚，导致置信度显示 19%/0%
  //   修复后: 置信度完全由公式决策计算，与 LLM 决策无关
  const rawConfidence = decision.confidence ?? 0;
  const confidencePct = Math.round(Math.max(0, Math.min(100, rawConfidence ?? 0)));
  const meterColor = confidencePct >= 70
    ? "var(--sa-green)"
    : confidencePct >= 45
    ? "var(--sa-amber)"
    : "var(--sa-red)";

  // V58: 决策方向置信度 — 当与 confidence 差异大时（看空决策被低 confidence 掩盖）展示
  const decisionConfidencePct = decision.decisionConfidence != null
    ? Math.round(decision.decisionConfidence)
    : null;
  const showDecisionConfidence = decisionConfidencePct != null
    && Math.abs(decisionConfidencePct - confidencePct) > 15;
  const decisionConfidenceColor = decisionConfidencePct != null
    ? (decisionConfidencePct >= 70
      ? "var(--sa-green)"
      : decisionConfidencePct >= 45
      ? "var(--sa-amber)"
      : "var(--sa-red)")
    : "var(--sa-amber)";

  // 置信度定性标签：让用户快速理解数字含义，而非只看到裸百分比
  const confidenceLabel = confidencePct >= 70
    ? t("stockAnalysis.confidenceHigh")
    : confidencePct >= 45
    ? t("stockAnalysis.confidenceMedium")
    : t("stockAnalysis.confidenceLow");
  const confidenceLabelColor = confidencePct >= 70
    ? "var(--sa-green)"
    : confidencePct >= 45
    ? "var(--sa-amber)"
    : "var(--sa-red)";

  // 从报价和决策计算预期收益
  const currentPrice = quote?.price ?? 0;
  const targetPriceNum = decision.targetPrice != null ? Number(decision.targetPrice) : 0;
  const upside = targetPriceNum > 0 && currentPrice > 0
    ? ((targetPriceNum - currentPrice) / currentPrice * 100)
    : null;

  // 时间范围标签（两种模式都可能用到）
  const timeHorizonLabel = decision.timeHorizon
    ? t(`stockAnalysis.timeHorizon${
      decision.timeHorizon === "ultra_short"
        ? "UltraShort"
        : decision.timeHorizon === "short"
        ? "Short"
        : decision.timeHorizon === "mid"
        ? "Mid"
        : "Long"
    }`)
    : null;

  // 阶段1（PROPOSAL-stock-decision-four-horizon.md）：四周期价位映射展示。
  // 遍历固定的四个键 → i18n 标签，仅在有值（stopLossPct > 0，即该周期有交易计划）时渲染。
  const HORIZON_KEYS: ReadonlyArray<{ key: "ultraShort" | "short" | "mid" | "long"; tSuffix: string }> = [
    { key: "ultraShort", tSuffix: "UltraShort" },
    { key: "short", tSuffix: "Short" },
    { key: "mid", tSuffix: "Mid" },
    { key: "long", tSuffix: "Long" },
  ];
  const horizonEntries = HORIZON_KEYS
    .map((h) => ({ h, group: decision.horizonPriceMap?.[h.key] ?? null }))
    .filter((e): e is { h: typeof e.h; group: HorizonPriceGroup } =>
      Boolean(e.group && e.group.stopLossPct > 0),
    );

  // 阶段2（PROPOSAL-stock-decision-four-horizon.md）：四周期独立决策面板。
  // 从 decisionsByHorizon 提取四组独立决策；仅在有数据（action 非空）的周期渲染。
  const horizonDecisionEntries = useMemo(() => {
    const dbh = decision.decisionsByHorizon;
    if (!dbh) { return []; }
    const entries: Array<{ key: "ultraShort" | "short" | "mid" | "long"; tSuffix: string; d: import("@/types").HorizonDecision }> = [];
    for (const { key, tSuffix } of HORIZON_KEYS) {
      const d = dbh[key];
      if (d && d.action) {
        entries.push({ key, tSuffix, d });
      }
    }
    return entries;
  }, [decision.decisionsByHorizon]);

  // 矛盾检测：四周期中同时存在 BUY 族与 SELL 族，或者两两方向冲突。
  const horizonHasConflict = useMemo(() => {
    if (horizonDecisionEntries.length < 2) { return false; }
    const dirs = horizonDecisionEntries.map((e) => actionToDirection(e.d.action));
    const hasBuy = dirs.some((d) => d === "buy");
    const hasSell = dirs.some((d) => d === "sell");
    return hasBuy && hasSell;
  }, [horizonDecisionEntries]);

  // 四周期标签页状态（默认选中第一个有数据的周期）
  const [activeHorizonTab, setActiveHorizonTab] = useState<string | null>(null);
  const defaultHorizonTab = horizonDecisionEntries.length > 0 ? horizonDecisionEntries[0].key : null;
  const currentHorizonTab = activeHorizonTab ?? defaultHorizonTab;
  // 当数据变化时重置标签页选中
  useEffect(() => {
    if (horizonDecisionEntries.length > 0 && !horizonDecisionEntries.some((e) => e.key === activeHorizonTab)) {
      setActiveHorizonTab(horizonDecisionEntries[0].key);
    }
  }, [horizonDecisionEntries, activeHorizonTab]);

  return (
    <>
      {/* ── 嵌入模式（工作区）：极简工具栏。顶部 DecisionHeroBar 已显示核心决策指标，避免重复 ── */}
      {embeddedInWorkspace && (
        <div
          className="flex items-center gap-2 flex-wrap py-1 px-2 rounded"
          style={{ background: "var(--surface)", minHeight: 32 }}
        >
          {/* 左侧：状态标签 + 补充指标（HeroBar 未显示的） */}
          <div className="flex items-center gap-1.5 flex-wrap">
            {asOfDate && (
              <Tag color="purple" style={{ margin: 0, fontSize: 11, lineHeight: "18px", paddingInline: 4 }}>
                ⏪ {t("timeTravel.pageAnchor.untilDate", { date: asOfDate })}
              </Tag>
            )}
            {isContradictory && (
              <Tag color="orange" style={{ margin: 0, fontSize: 11, lineHeight: "18px", paddingInline: 4 }}>
                ⚠️ {t("stockAnalysis.contradiction")}
              </Tag>
            )}
            {timeHorizonLabel && (
              <Tag color="geekblue" style={{ margin: 0, fontSize: 11, lineHeight: "18px", paddingInline: 4 }}>
                {timeHorizonLabel}
              </Tag>
            )}
            {
              /* 决策可信度受限（权重坍缩 + 数据缺口）：由共享组件统一渲染。
                原实现只覆盖 weightsCollapsed，且该字段被 normalizeDecision 的
                白名单丢弃，实际从未显示过（2026-09-11 修复）。 */
            }
            <DecisionTrustNotice decision={decision} variant="tag" />
            {decision.expectedHoldingDays && (
              <span className="text-xs font-mono" style={{ color: "var(--muted)" }}>
                {t("stockAnalysis.expectedHoldingDaysLabel")}{" "}
                <span style={{ color: "var(--color-text-primary)", fontWeight: 500 }}>
                  {t("stockAnalysis.expectedHoldingDays", { days: decision.expectedHoldingDays })}
                </span>
              </span>
            )}
            {decision.targetTimeframe && (
              <span className="text-xs font-mono" style={{ color: "var(--muted)" }}>
                {t("stockAnalysis.targetTimeframe")}{" "}
                <span style={{ color: "var(--color-text-primary)", fontWeight: 500 }}>
                  {decision.targetTimeframe}
                </span>
              </span>
            )}
          </div>

          {/* 右侧：操作按钮 */}
          <div className="flex items-center gap-1 ml-auto flex-wrap">
            {stockCode && !watchlisted && (
              <Button
                size="small"
                type="dashed"
                loading={adding}
                onClick={addToWatchlist}
                style={{ fontSize: 11, height: 24, padding: "0 6px" }}
              >
                ⭐ {t("stockAnalysis.addToWatchlist")}
              </Button>
            )}
            {watchlisted && (
              <Tag color="gold" style={{ margin: 0, fontSize: 11, lineHeight: "20px", paddingInline: 4 }}>
                ⭐ {t("stockAnalysis.inWatchlist")}
              </Tag>
            )}
            {stockCode && (
              <>
                <Button
                  size="small"
                  icon={<ReloadOutlined style={{ fontSize: 11 }} />}
                  onClick={() => {
                    useStockAnalysisStore.getState().startAnalysis(
                      stockCode,
                      analysisId ? { parentAnalysisId: analysisId } : undefined,
                    );
                  }}
                  style={{ fontSize: 11, height: 24, padding: "0 6px" }}
                >
                  {t("stockAnalysis.reAnalyze")}
                </Button>
                <Tooltip title={t("stockAnalysis.rerunDecisionHint")}>
                  <Button
                    size="small"
                    icon={<span style={{ fontSize: 11 }}>⚡</span>}
                    onClick={() => {
                      if (analysisId) {
                        useStockAnalysisStore.getState().rerunDecision(analysisId);
                      }
                    }}
                    style={{ fontSize: 11, height: 24, padding: "0 6px" }}
                  >
                    {t("stockAnalysis.rerunDecision")}
                  </Button>
                </Tooltip>
                <Button
                  size="small"
                  icon={<span style={{ fontSize: 11 }}>💬</span>}
                  onClick={handleAskAI}
                  style={{ fontSize: 11, height: 24, padding: "0 6px" }}
                >
                  {t("stockAnalysis.askAI")}
                </Button>
                <Dropdown
                  menu={{
                    items: exportMenuItems.map((item) => ({
                      key: item.key,
                      icon: item.icon,
                      label: item.label,
                      disabled: exporting === item.key,
                      onClick: () => handleExport(item.key),
                    })),
                  }}
                  trigger={["click"]}
                >
                  <Button
                    size="small"
                    loading={exporting !== null}
                    icon={<span style={{ fontSize: 11 }}>📥</span>}
                    style={{ fontSize: 11, height: 24, padding: "0 6px" }}
                  >
                    {t("stockAnalysis.exportReport")}
                  </Button>
                </Dropdown>
                <Button
                  size="small"
                  type="text"
                  onClick={() => setExpanded(true)}
                  style={{ fontSize: 11, height: 24, padding: "0 4px", color: "var(--accent)" }}
                >
                  ▶ {t("stockAnalysis.showDetail")}
                </Button>
              </>
            )}
          </div>
        </div>
      )}

      {/* ── 独立模式（/stock-analysis 路由）：完整 Card + 绿色一致性条 + 内联对比面板 ── */}
      {!embeddedInWorkspace && (
        <>
          <Card
            id="decision-banner-top"
            size="small"
            title={decisionAgreementScore !== null && decisionAgreementScore < disagreementThreshold && llmStance
              ? (
                /* 分歧时：双决策并列标题 */
                <div className="flex items-center gap-2 flex-wrap">
                  <span className="text-sm">{t("stockAnalysis.dualViewDisagreementTitle")}</span>
                  <Tag color={getActionColor(displayAction ?? "")}>
                    {t("stockAnalysis.formula")} {actionLabel(displayAction ?? "")}
                  </Tag>
                  <span className="text-sm" style={{ color: "var(--muted)" }}>vs</span>
                  <Tag color={getActionColor(llmStance)}>
                    LLM {t(getActionTKey(llmStance))}
                  </Tag>
                  {asOfDate && (
                    <Tag color="purple" title={t("timeTravel.badge.replayTooltip", { date: asOfDate })}>
                      ⏪ {t("timeTravel.pageAnchor.untilDate", { date: asOfDate })}
                    </Tag>
                  )}
                  {isContradictory && (
                    <Tag color="orange">
                      ⚠️ {t("stockAnalysis.contradiction")}
                    </Tag>
                  )}
                </div>
              )
              : (
                /* 一致或无LLM时：原单决策标题 */
                <div className="flex items-center gap-2">
                  <span>{t("stockAnalysis.finalDecision")}</span>
                  <Tag color={getActionColor(displayAction ?? "")}>
                    {actionLabel(displayAction ?? "")}
                  </Tag>
                  {asOfDate && (
                    <Tag color="purple" title={t("timeTravel.badge.replayTooltip", { date: asOfDate })}>
                      ⏪ {t("timeTravel.pageAnchor.untilDate", { date: asOfDate })}
                    </Tag>
                  )}
                  {isContradictory && (
                    <Tag color="orange">
                      ⚠️ {t("stockAnalysis.contradiction")}
                    </Tag>
                  )}
                  {decision.timeHorizon && (
                    <Tag color="geekblue">
                      {t(`stockAnalysis.timeHorizon${
                        decision.timeHorizon === "ultra_short"
                          ? "UltraShort"
                          : decision.timeHorizon === "short"
                          ? "Short"
                          : decision.timeHorizon === "mid"
                          ? "Mid"
                          : "Long"
                      }`)}
                    </Tag>
                  )}
                  {/* 决策可信度受限：标题行用紧凑标签，完整警示条在下方正文区 */}
                  <DecisionTrustNotice decision={decision} variant="tag" />
                </div>
              )}
            extra={
              <Button
                type="text"
                size="small"
                icon={<ExpandOutlined />}
                onClick={() => setExpanded(true)}
              />
            }
            styles={{ body: { padding: "12px 18px" } }}
            style={{
              borderLeft: "4px solid var(--accent)",
              ...(highlightedPanel === "decision"
                ? { boxShadow: "0 0 0 3px var(--accent)", transition: "box-shadow 0.4s" }
                : {}),
            }}
          >
            {
              /* 决策可信度受限（权重坍缩 / 数据缺口）—— 正文首行，把「为什么观望」
                的前提摊开，避免只给结论不给依据的「黑盒观望」体感 */
            }
            {(decision.weightsCollapsed || (decision.dataGaps?.length ?? 0) > 0) && (
              <div className="mb-2">
                <DecisionTrustNotice decision={decision} variant="banner" />
              </div>
            )}

            {/* 置信度条：数字 + 定性标签 + 共识上下文 — 嵌入模式下精简（顶部HeroBar已显示） */}
            {!embeddedInWorkspace && (
              <div className="mb-2">
                <div className="flex justify-between items-center text-sm mb-1">
                  <div className="flex items-center gap-1.5">
                    <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.confidence")}</span>
                    <span
                      className="font-mono font-semibold"
                      style={{ color: meterColor, fontSize: 18 }}
                    >
                      {confidencePct}%
                    </span>
                    {/* V68 修复: 移除 adjustedConfidence 显示 */}
                    <span
                      className="text-sm px-1.5 py-px rounded font-medium"
                      style={{
                        background: `${confidenceLabelColor}18`,
                        color: confidenceLabelColor,
                        border: `1px solid ${confidenceLabelColor}40`,
                      }}
                    >
                      {confidenceLabel}
                    </span>
                    {/* V66 修复(2026-07-29): 补充 confidence 语义说明，与 decisionConfidence 区分 */}
                    <Tooltip title={t("stockAnalysis.confidenceBayesianTooltip")}>
                      <span className="text-xs ml-1 cursor-help" style={{ color: "var(--muted)" }}>?</span>
                    </Tooltip>
                    {/* V58: 决策方向置信度 — 看空决策 confidence 低但 decision_confidence 高时展示 */}
                    {showDecisionConfidence && decisionConfidencePct != null && (
                      <Tooltip title={t("stockAnalysis.decisionConfidenceTooltip")}>
                        <span
                          className="text-sm px-1.5 py-px rounded font-medium cursor-help"
                          style={{
                            background: `${decisionConfidenceColor}18`,
                            color: decisionConfidenceColor,
                            border: `1px solid ${decisionConfidenceColor}40`,
                          }}
                        >
                          {t("stockAnalysis.decisionConfidence")}: {decisionConfidencePct}%
                        </span>
                      </Tooltip>
                    )}
                  </div>
                </div>
                <div
                  className="relative"
                  style={{ height: 6, borderRadius: 3, background: "var(--surface)", overflow: "hidden" }}
                >
                  <div
                    style={{
                      width: `${confidencePct}%`,
                      height: "100%",
                      borderRadius: 3,
                      background: `linear-gradient(to right, ${meterColor}88, ${meterColor})`,
                      transition: "width 0.6s ease",
                    }}
                  />
                </div>
              </div>
            )}
            {/* 嵌入模式下仅显示定性标签（不重复数字/进度条） */}
            {embeddedInWorkspace && (
              <div className="mb-2 flex items-center gap-2">
                <span
                  className="text-sm px-2 py-0.5 rounded font-medium"
                  style={{
                    background: `${confidenceLabelColor}18`,
                    color: confidenceLabelColor,
                    border: `1px solid ${confidenceLabelColor}40`,
                  }}
                >
                  {confidenceLabel}
                </span>
                {showDecisionConfidence && decisionConfidencePct != null && (
                  <Tooltip title={t("stockAnalysis.decisionConfidenceTooltip")}>
                    <span
                      className="text-sm px-1.5 py-px rounded font-medium cursor-help"
                      style={{
                        background: `${decisionConfidenceColor}18`,
                        color: decisionConfidenceColor,
                        border: `1px solid ${decisionConfidenceColor}40`,
                      }}
                    >
                      {t("stockAnalysis.decisionConfidence")}: {decisionConfidencePct}%
                    </span>
                  </Tooltip>
                )}
              </div>
            )}

            {/* V50+: 分歧时内联双视角对比卡片（紧凑版） */}
            {decisionAgreementScore !== null && decisionAgreementScore < disagreementThreshold && llmStance && (
              <div
                className="mb-2 p-1.5 rounded space-y-1"
                style={{ background: "var(--surface)", border: "1px solid var(--border)" }}
              >
                <div className="flex items-center justify-between">
                  <span className="text-sm font-semibold" style={{ color: "#7c3aed" }}>
                    📊 {t("stockAnalysis.dualViewComparisonTitle")}
                  </span>
                  <span
                    className="text-sm font-mono px-1 py-px rounded"
                    style={{ background: "rgba(239,68,68,0.10)", color: "#ef4444" }}
                  >
                    {decisionAgreementScore}/100
                  </span>
                </div>
                <div className="grid grid-cols-2 gap-1.5 text-sm">
                  {/* 公式视角 */}
                  <div
                    className="rounded px-1.5 py-1 space-y-0.5"
                    style={{ background: "rgba(37,99,235,0.05)", borderLeft: "2px solid #2563eb" }}
                  >
                    <div className="flex items-center justify-between">
                      <span className="font-medium" style={{ color: "#2563eb" }}>{t("stockAnalysis.formula")}</span>
                      <Tag
                        color={getActionColor(displayAction ?? "")}
                        style={{ fontSize: 12, lineHeight: "20px", height: 20, paddingInline: 6 }}
                      >
                        {actionLabel(displayAction ?? "")}
                      </Tag>
                    </div>
                    <div className="font-mono flex gap-2" style={{ color: "var(--color-text-secondary)" }}>
                      <span>
                        {t("stockAnalysis.confidence")} <b>{confidencePct}%</b>
                      </span>
                      <span>|</span>
                      <span>
                        {t("stockAnalysis.position")} <b>{decision.positionPct}%</b>
                      </span>
                    </div>
                    {decision.reasoning && (
                      <div
                        className="line-clamp-1"
                        style={{ color: "var(--muted)", fontSize: "12px" }}
                        title={cleanToolCallTags(decision.reasoning) ?? ""}
                      >
                        {cleanToolCallTags(decision.reasoning)?.slice(0, 50)}
                        {(cleanToolCallTags(decision.reasoning)?.length ?? 0) > 50 ? "…" : ""}
                      </div>
                    )}
                  </div>
                  {/* LLM 视角 */}
                  <div
                    className="rounded px-1.5 py-1 space-y-0.5"
                    style={{ background: "rgba(124,58,237,0.05)", borderLeft: "2px solid #7c3aed" }}
                  >
                    <div className="flex items-center justify-between">
                      <span className="font-medium" style={{ color: "#7c3aed" }}>LLM</span>
                      <div className="flex items-center gap-1">
                        <Tag
                          color={getActionColor(llmStance)}
                          style={{ fontSize: 12, lineHeight: "20px", height: 20, paddingInline: 6 }}
                        >
                          {t(getActionTKey(llmStance))}
                        </Tag>
                        {(() => {
                          const llmConf = extractLlmField(llmDecisionJson, "confidence") as number | null;
                          if (llmConf != null && Math.round(llmConf) > confidencePct) {
                            return (
                              <span
                                className="text-sm px-0.5 rounded font-medium"
                                style={{ background: "rgba(16,185,129,0.15)", color: "#10b981" }}
                              >
                                ✓ {t("stockAnalysis.recommended")}
                              </span>
                            );
                          }
                          return null;
                        })()}
                      </div>
                    </div>
                    <div className="font-mono flex gap-2" style={{ color: "var(--color-text-secondary)" }}>
                      <span>
                        {t("stockAnalysis.confidence")}{" "}
                        <b>
                          {(() => {
                            const c = extractLlmField(llmDecisionJson, "confidence") as number | null;
                            return c != null ? `${Math.round(c)}%` : "—";
                          })()}
                        </b>
                      </span>
                      <span>|</span>
                      <span>
                        {t("stockAnalysis.position")}{" "}
                        <b>
                          {(() => {
                            const p = extractLlmField(llmDecisionJson, "positionPct") as number | null;
                            return p != null ? `${Math.round(p)}%` : "—";
                          })()}
                        </b>
                      </span>
                    </div>
                    {llmSummary && (
                      <div
                        className="line-clamp-1"
                        style={{ color: "var(--muted)", fontSize: "12px" }}
                        title={llmSummary}
                      >
                        {llmSummary.slice(0, 50)}…
                      </div>
                    )}
                  </div>
                </div>
              </div>
            )}

            {/* Reasoning: collapsed in banner, detailed in Modal via expand button */}
            <div
              className="text-sm mb-2 p-2 rounded cursor-pointer hover:opacity-80 transition-opacity"
              style={{
                background: "var(--surface)",
                maxWidth: "100%",
                minWidth: 0,
                overflowWrap: "anywhere",
                wordBreak: "break-all",
              }}
              onClick={() => setExpanded(true)}
              role="button"
              tabIndex={0}
              onKeyDown={(e) => {
                if (e.key === "Enter") { setExpanded(true); }
              }}
            >
              {cleanToolCallTags(decision.reasoning || "")
                ? (
                  <>
                    <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.reasoning")}</span>
                    <span
                      className="ml-1"
                      style={{
                        color: "var(--color-text-secondary)",
                        overflowWrap: "anywhere",
                        wordBreak: "break-all",
                      }}
                    >
                      {cleanToolCallTags(decision.reasoning!)?.slice(0, embeddedInWorkspace ? 60 : 120)}
                      {cleanToolCallTags(decision.reasoning!)!.length > (embeddedInWorkspace ? 60 : 120) ? "…" : ""}
                    </span>
                    <span className="ml-1 text-sm" style={{ color: "var(--accent)" }}>
                      ▶ {t("stockAnalysis.showDetail")}
                    </span>
                  </>
                )
                : <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.noDecisionReasoning")}</span>}
            </div>

            {/* [Phase 2 step 10] 分歧时 LLM 对比标注 */}
            {decisionAgreementScore !== null && decisionAgreementScore < disagreementThreshold && llmSummary && (
              <div
                className="text-sm mb-2 p-2 rounded"
                style={{ background: "rgba(124, 58, 237, 0.08)", borderLeft: "3px solid #7c3aed" }}
              >
                <span className="font-medium" style={{ color: "#7c3aed" }}>
                  💡 {t("stockAnalysis.llmPerspective")}:
                </span>
                <span className="ml-1" style={{ color: "var(--color-text-secondary)" }}>
                  {llmSummary}
                </span>
              </div>
            )}

            {
              /* 决策依据说明书（decision-explainer 节点产出）——
                与上方 llmSummary 的区别：llmSummary 是 trader 的 LLM 视角（方案 D 对比用），
                本块是决策解释官对**风控门最终裁决**的翻译 + 规则追溯码清单。
                无产出时组件自渲染为空（该节点 continue_on_fail=true，跳过属预期）。 */
            }
            <div className="mb-2">
              <DecisionExplanationPanel />
            </div>

            {/* 紧凑指标行：grid 分列填满 Card 宽度，避免右侧留空 */}
            <div className="grid gap-1.5 mb-2" style={{ gridTemplateColumns: "repeat(auto-fill, minmax(160px, 1fr))" }}>
              {decision.targetPrice && (
                <span
                  className="text-sm px-2 py-1 rounded font-mono flex items-center justify-between"
                  style={{ background: "var(--surface)", color: "var(--color-text-primary)" }}
                >
                  <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.targetPrice")}</span>
                  <span className="font-semibold">¥{decision.targetPrice}</span>
                </span>
              )}
              {decision.stopLoss && (
                <span
                  className="text-sm px-2 py-1 rounded font-mono flex items-center justify-between"
                  style={{ background: "var(--surface)", color: "var(--sa-red)" }}
                >
                  <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.stopLoss")}</span>
                  <span className="font-semibold">¥{decision.stopLoss}</span>
                </span>
              )}
              <span
                className="text-sm px-2 py-1 rounded font-mono flex items-center justify-between"
                style={{ background: "var(--surface)", color: "var(--color-text-primary)" }}
              >
                <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.position")}</span>
                <span className="font-semibold">{decision.positionPct}%</span>
              </span>
              {upside != null && (
                <span
                  className="text-sm px-2 py-1 rounded font-mono flex items-center justify-between"
                  style={{
                    background: "var(--surface)",
                    color: upside >= 0 ? "var(--sa-green)" : "var(--sa-red)",
                  }}
                >
                  <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.expectedUpside")}</span>
                  <span className="font-semibold">{upside >= 0 ? "+" : ""}{upside.toFixed(1)}%</span>
                </span>
              )}
              <span
                className="text-sm px-2 py-1 rounded flex items-center justify-between"
                style={{ background: "var(--surface)", color: getRiskColor(decision.riskLevel) }}
              >
                <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.riskLevel")}</span>
                <span className="font-semibold">{t(getRiskTKey(decision.riskLevel))}</span>
              </span>
              {/* P0-1: 证据质量门控显示 */}
              {stockCodeEvidence && stockCodeEvidence.holdGate && (
                <span
                  className="text-sm px-2 py-1 rounded flex items-center justify-between"
                  style={{
                    background: "var(--surface)",
                    color: stockCodeEvidence.holdGate.holdAllowed ? "var(--sa-blue)" : "var(--sa-amber)",
                  }}
                >
                  <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.decision.gateControl")}</span>
                  <span className="font-semibold text-sm">
                    {stockCodeEvidence.holdGate.holdAllowed
                      ? "✅ HOLD"
                      : t("stockAnalysis.decision.mandatoryDirection")}
                  </span>
                </span>
              )}
              {stockCodeEvidence && (
                <span
                  className="text-sm px-2 py-1 rounded flex items-center justify-between font-mono"
                  style={{ background: "var(--surface)", color: "var(--color-text-primary)" }}
                >
                  <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.decision.evidenceScore")}</span>
                  <span className="font-semibold">
                    {t("stockAnalysis.debate.bullish")}
                    {stockCodeEvidence.consensus.bullishScore.toFixed(1)} | {t("stockAnalysis.debate.bearish")}
                    {stockCodeEvidence.consensus.bearishScore.toFixed(1)} | {t("stockAnalysis.decision.net")}
                    {stockCodeEvidence.consensus.netScore.toFixed(1)}
                  </span>
                </span>
              )}
              {decision.expectedHoldingDays && (
                <span
                  className="text-sm px-2 py-1 rounded font-mono flex items-center justify-between"
                  style={{ background: "var(--surface)", color: "var(--color-text-primary)" }}
                >
                  <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.expectedHoldingDaysLabel")}</span>
                  <span className="font-semibold">
                    {t("stockAnalysis.expectedHoldingDays", { days: decision.expectedHoldingDays })}
                  </span>
                </span>
              )}
              {decision.targetTimeframe && (
                <span
                  className="text-sm px-2 py-1 rounded font-mono flex items-center justify-between"
                  style={{ background: "var(--surface)", color: "var(--color-text-primary)" }}
                >
                  <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.targetTimeframe")}</span>
                  <span className="font-semibold">{decision.targetTimeframe}</span>
                </span>
              )}
            </div>

            {/* 阶段1：四周期价位映射展示（PROPOSAL-stock-decision-four-horizon.md）
              同一决策保留单一 action/仓位，但目标价/止损按四周期各给一组。
              仅在有交易计划（stopLossPct>0）的周期渲染；无映射时整体隐藏。
              标签/文案复用既有 key（timeHorizonX / targetPrice / stopLoss /
              expectedHoldingDays），不新增 i18n key。 */}
            {horizonEntries.length > 0 && (
              <div className="mb-2">
                <div className="grid gap-1.5" style={{ gridTemplateColumns: "repeat(auto-fill, minmax(200px, 1fr))" }}>
                  {horizonEntries.map(({ h, group }) => {
                    const groupTarget = Number(group.targetPrice);
                    const groupStop = Number(group.stopLoss);
                    return (
                      <div
                        key={h.key}
                        className="text-sm px-2 py-1 rounded"
                        style={{ background: "var(--surface)", color: "var(--color-text-primary)" }}
                      >
                        <div className="flex items-center justify-between mb-1">
                          <span className="font-semibold">{t(`stockAnalysis.timeHorizon${h.tSuffix}`)}</span>
                          <span style={{ color: "var(--muted)" }}>
                            {t("stockAnalysis.expectedHoldingDays", { days: group.expectedHoldingDays })}
                          </span>
                        </div>
                        <div className="flex items-center justify-between font-mono">
                          <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.targetPrice")}</span>
                          <span className="font-semibold">{groupTarget > 0 ? `¥${groupTarget}` : "—"}</span>
                        </div>
                        <div className="flex items-center justify-between font-mono">
                          <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.stopLoss")}</span>
                          <span className="font-semibold" style={{ color: "var(--sa-red)" }}>
                            {groupStop > 0 ? `¥${groupStop}` : "—"}
                          </span>
                        </div>
                      </div>
                    );
                  })}
                </div>
              </div>
            )}

            {/* 阶段2：四周期独立决策面板（PROPOSAL-stock-decision-four-horizon.md）
              仅在有 decisionsByHorizon 数据时渲染。每个周期独立产出 action/仓位/目标价/止损，
              按周期 Tab 分组展示。方向矛盾时高亮呈现（验收项）。 */}
            {horizonDecisionEntries.length > 0 && (
              <div className="mb-2">
                {/* 方向矛盾警告条 */}
                {horizonHasConflict && (
                  <div
                    className="text-sm px-2 py-1 rounded mb-1.5 flex items-center gap-1.5"
                    style={{
                      background: "rgba(239,68,68,0.08)",
                      borderLeft: "3px solid var(--sa-red, #ef4444)",
                    }}
                  >
                    <span>⚠️</span>
                    <span className="font-medium" style={{ color: "var(--sa-red, #ef4444)" }}>
                      {t("stockAnalysis.horizonConflictTitle")}
                    </span>
                    <span style={{ color: "var(--muted)" }}>
                      {t("stockAnalysis.horizonConflictHint")}
                    </span>
                  </div>
                )}
                {/* 周期 Tab 切换 */}
                <div className="flex items-center gap-1 mb-1.5 flex-wrap">
                  {horizonDecisionEntries.map((e) => {
                    const isActive = e.key === currentHorizonTab;
                    const dir = actionToDirection(e.d.action);
                    const tabColor = dir === "buy"
                      ? "var(--sa-green, #10b981)"
                      : dir === "sell"
                      ? "var(--sa-red, #ef4444)"
                      : "var(--muted)";
                    return (
                      <button
                        key={e.key}
                        onClick={() => setActiveHorizonTab(e.key)}
                        className="text-sm px-2 py-0.5 rounded font-medium transition-colors"
                        style={{
                          background: isActive
                            ? `${tabColor}22`
                            : "var(--surface)",
                          color: isActive ? tabColor : "var(--muted)",
                          border: isActive
                            ? `1px solid ${tabColor}66`
                            : "1px solid var(--border)",
                          cursor: "pointer",
                        }}
                      >
                        {t(`stockAnalysis.timeHorizon${e.tSuffix}`)}
                        <span
                          className="ml-1 px-1 py-px rounded text-xs font-bold"
                          style={{
                            background: `${tabColor}22`,
                            color: tabColor,
                          }}
                        >
                          {t(getActionTKey(e.d.action))}
                        </span>
                      </button>
                    );
                  })}
                </div>
                {/* 当前选中周期的决策详情 */}
                {currentHorizonTab && (() => {
                  const activeEntry = horizonDecisionEntries.find((e) => e.key === currentHorizonTab);
                  if (!activeEntry) { return null; }
                  const { d } = activeEntry;
                  const dir = actionToDirection(d.action);
                  const hConf = Math.round(Math.max(0, Math.min(100, d.confidence ?? 0)));
                  const hConfColor = hConf >= 70 ? "var(--sa-green)" : hConf >= 45 ? "var(--sa-amber)" : "var(--sa-red)";
                  const hTarget = d.targetPrice != null ? Number(d.targetPrice) : null;
                  const hStop = d.stopLoss != null ? Number(d.stopLoss) : null;
                  return (
                    <div
                      className="rounded px-2 py-1.5 space-y-1"
                      style={{
                        background: "var(--surface)",
                        borderLeft: `3px solid ${
                          dir === "buy" ? "var(--sa-green)" : dir === "sell" ? "var(--sa-red)" : "var(--muted)"
                        }`,
                      }}
                    >
                      {/* 行 1：仓位 + 置信度 + 预期持仓 */}
                      <div className="flex items-center gap-2 text-sm flex-wrap">
                        <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.position")}</span>
                        <span className="font-semibold font-mono">{d.positionPct}%</span>
                        <span className="text-sm" style={{ color: "var(--muted)" }}>·</span>
                        <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.confidence")}</span>
                        <span className="font-mono font-semibold" style={{ color: hConfColor }}>{hConf}%</span>
                        {d.expectedHoldingDays > 0 && (
                          <>
                            <span className="text-sm" style={{ color: "var(--muted)" }}>·</span>
                            <span style={{ color: "var(--muted)" }}>
                              {t("stockAnalysis.expectedHoldingDays", { days: d.expectedHoldingDays })}
                            </span>
                          </>
                        )}
                      </div>
                      {/* 行 2：目标价 + 止损 */}
                      <div className="flex items-center gap-2 text-sm flex-wrap">
                        <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.targetPrice")}</span>
                        <span className="font-mono font-semibold">
                          {hTarget != null && hTarget > 0 ? `¥${hTarget}` : "—"}
                        </span>
                        <span className="text-sm" style={{ color: "var(--muted)" }}>·</span>
                        <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.stopLoss")}</span>
                        <span className="font-mono font-semibold" style={{ color: "var(--sa-red)" }}>
                          {hStop != null && hStop > 0 ? `¥${hStop}` : "—"}
                        </span>
                        {d.takeProfitPct > 0 && (
                          <>
                            <span className="text-sm" style={{ color: "var(--muted)" }}>·</span>
                            <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.takeProfit")}</span>
                            <span className="font-mono font-semibold" style={{ color: "var(--sa-green)" }}>
                              +{d.takeProfitPct}%
                            </span>
                          </>
                        )}
                        {d.stopLossPct > 0 && (
                          <>
                            <span className="text-sm" style={{ color: "var(--muted)" }}>·</span>
                            <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.stopLoss")}%</span>
                            <span className="font-mono font-semibold" style={{ color: "var(--sa-red)" }}>
                              -{d.stopLossPct}%
                            </span>
                          </>
                        )}
                      </div>
                      {/* 行 3：verdict 简述（若有） */}
                      {d.verdict && (
                        <div className="text-sm" style={{ color: "var(--color-text-secondary)" }}>
                          {d.verdict}
                        </div>
                      )}
                      {/* 超短线 confLowerBound 标注 */}
                      {d.confLowerBound != null && (
                        <div className="text-xs" style={{ color: "var(--muted)" }}>
                          {t("stockAnalysis.confLowerBoundHint", { bound: d.confLowerBound })}
                        </div>
                      )}
                    </div>
                  );
                })()}
                {/* 概览条：四周期 action 一览 */}
                <div className="flex items-center gap-1.5 text-xs mt-1" style={{ color: "var(--muted)" }}>
                  <span>{t("stockAnalysis.horizonOverview")}</span>
                  {horizonDecisionEntries.map((e) => {
                    const dir = actionToDirection(e.d.action);
                    return (
                      <span
                        key={e.key}
                        className="px-1.5 py-px rounded font-medium"
                        style={{
                          background: dir === "buy"
                            ? "rgba(16,185,129,0.12)"
                            : dir === "sell"
                            ? "rgba(239,68,68,0.12)"
                            : "rgba(156,163,175,0.12)",
                          color: dir === "buy" ? "var(--sa-green)" : dir === "sell" ? "var(--sa-red)" : "var(--muted)",
                          cursor: "pointer",
                        }}
                        onClick={() => setActiveHorizonTab(e.key)}
                      >
                        {t(`stockAnalysis.timeHorizon${e.tSuffix}`)}: {t(getActionTKey(e.d.action))}
                      </span>
                    );
                  })}
                </div>
              </div>
            )}

            {/* 操作按钮行 */}
            <div className="flex gap-2 items-center flex-wrap">
              {/* 快速交易录入（紧凑形式）— 决策日可直接在此录入买卖；嵌入模式下隐藏（工作区有专门交易Tab） */}
              {
                /* P0-1: 仅方向明确的决策才显示入口（持有/观望/不确定/数据缺失无交易方向，
                  显示一个点了必然报错的按钮没有意义），判定与提交处同源。 */
              }
              {stockCode && actionToDirection(decision.action) !== null && !embeddedInWorkspace && (
                <div className="flex items-center gap-1" style={{ marginRight: 4 }}>
                  <input
                    type="number"
                    placeholder={t("trade.price")}
                    defaultValue={decision.targetPrice ?? undefined}
                    ref={tradePriceRef}
                    className="text-sm"
                    style={{
                      width: 60,
                      padding: "1px 4px",
                      border: "1px solid var(--color-border-tertiary)",
                      borderRadius: 4,
                      background: "transparent",
                      color: "var(--color-text-primary)",
                      height: 24,
                    }}
                  />
                  <input
                    type="number"
                    placeholder={t("trade.quantity")}
                    defaultValue={100}
                    ref={tradeQtyRef}
                    className="text-sm"
                    style={{
                      width: 50,
                      padding: "1px 4px",
                      border: "1px solid var(--color-border-tertiary)",
                      borderRadius: 4,
                      background: "transparent",
                      color: "var(--color-text-primary)",
                      height: 24,
                    }}
                  />
                  <Button
                    size="small"
                    type="primary"
                    style={{ fontSize: 11, lineHeight: "20px", height: 24, padding: "0 6px" }}
                    onClick={async () => {
                      const price = parseFloat(tradePriceRef.current?.value ?? "0");
                      const qty = parseInt(tradeQtyRef.current?.value ?? "100", 10);
                      if (price <= 0 || qty <= 0) {
                        return;
                      }
                      // P0-1(2026-09-14): 方向必须由 action 归一化映射得出；
                      // 无方向的决策（持有 / 观望 / 不确定 / 数据缺失）拒绝录入。
                      // 原实现 `action === "SELL" ? "sell" : "buy"` 会把「观望」直接录成
                      // **买入方向**并写入真实交易记录（与 TradePanel 同型缺陷，第二入口）。
                      const direction = actionToDirection(decision.action);
                      if (!direction) {
                        message.warning(t("stockAnalysis.trade.quickRecordNoDirection"));
                        return;
                      }
                      const analysisId = useStockAnalysisStore.getState().analysisId;
                      try {
                        await invoke("record_trade", {
                          stockCode,
                          stockName,
                          direction,
                          price,
                          quantity: Math.round(qty / 100) * 100,
                          tradeDate: new Date().toISOString().slice(0, 10),
                          tradeTime: new Date().toISOString().slice(11, 16),
                          notes: `${t("stockAnalysis.trade.fromDecision")} (${
                            t("stockAnalysis.confidence")
                          }: ${confidencePct}%)`,
                          analysisId: analysisId ?? null,
                        });
                        message.success(t("trade.recorded"));
                      } catch (e: unknown) {
                        showBackendError(message, e);
                      }
                    }}
                  >
                    {t("stockAnalysis.trade.record")}
                  </Button>
                </div>
              )}
              {stockCode && !watchlisted && (
                <Button size="small" type="dashed" loading={adding} onClick={addToWatchlist}>
                  ⭐ {t("stockAnalysis.addToWatchlist")}
                </Button>
              )}
              {watchlisted && <Tag color="gold">⭐ {t("stockAnalysis.inWatchlist")}</Tag>}
              {stockCode && (
                <>
                  <Button
                    size="small"
                    icon={<ReloadOutlined />}
                    onClick={() => {
                      useStockAnalysisStore.getState().startAnalysis(
                        stockCode,
                        analysisId ? { parentAnalysisId: analysisId } : undefined,
                      );
                    }}
                  >
                    {t("stockAnalysis.reAnalyze")}
                  </Button>
                  <Tooltip title={t("stockAnalysis.rerunDecisionHint")}>
                    <Button
                      size="small"
                      icon={<span>⚡</span>}
                      onClick={() => {
                        if (analysisId) {
                          useStockAnalysisStore.getState().rerunDecision(analysisId);
                        }
                      }}
                    >
                      {t("stockAnalysis.rerunDecision")}
                    </Button>
                  </Tooltip>
                  <Button size="small" icon={<span>💬</span>} onClick={handleAskAI}>
                    {t("stockAnalysis.askAI")}
                  </Button>
                  <Dropdown
                    menu={{
                      items: exportMenuItems.map((item) => ({
                        key: item.key,
                        icon: item.icon,
                        label: item.label,
                        disabled: exporting === item.key,
                        onClick: () => handleExport(item.key),
                      })),
                    }}
                    trigger={["click"]}
                  >
                    <Button size="small" loading={exporting !== null} icon={<span>📥</span>}>
                      {t("stockAnalysis.exportReport")}
                    </Button>
                  </Dropdown>
                </>
              )}
            </div>
          </Card>

          {/* [Phase 2] 决策一致性胶囊: 点击展开双视角对比 — 嵌入模式下隐藏（顶部HeroBar已显示） */}
          {!embeddedInWorkspace && decisionAgreementScore !== null && (
            <div
              className="flex items-center gap-2 px-3 py-1 rounded cursor-pointer hover:opacity-80 transition-opacity mt-0.5"
              style={{
                // 2026-09-21: 走单一真相源（原内联 60/40 档位表）
                background: agreementBgColor(decisionAgreementScore),
              }}
              onClick={() => {
                // V50+: 分歧时内联展开对比面板，一致时跳转 tab 查看详情
                if (decisionAgreementScore !== null && decisionAgreementScore < disagreementThreshold) {
                  setShowInlineComparison(!showInlineComparison);
                } else {
                  window.dispatchEvent(new CustomEvent("switch-tab", { detail: "decision-comparison" }));
                }
              }}
            >
              <span className="text-sm" style={{ color: "var(--muted)" }}>
                📊 {t("stockAnalysis.dualViewConsistency")}
              </span>
              <span
                className="font-mono text-[12px] font-semibold"
                style={{ color: agreementColor(decisionAgreementScore) }}
              >
                {decisionAgreementScore}/100
              </span>
              {llmStance && (
                <>
                  <span className="text-sm" style={{ color: "var(--muted)" }}>·</span>
                  <span
                    className="px-1.5 rounded text-sm font-medium"
                    style={{ background: "var(--sa-purple-bg, #ede9fe)", color: "#7c3aed" }}
                  >
                    LLM: {llmStance}
                  </span>
                </>
              )}
              {decisionAgreementScore < disagreementThreshold && (
                <span className="text-sm" style={{ color: "#ef4444" }}>
                  ⚠️ {t("stockAnalysis.dualViewDisagreement")}
                </span>
              )}
            </div>
          )}

          {/* V50+: 内联展开完整双视角对比面板（紧凑版）— 嵌入模式下隐藏 */}
          {!embeddedInWorkspace && showInlineComparison && decisionAgreementScore !== null && (
            <div
              className="mt-1.5 p-1.5 rounded space-y-1"
              style={{ background: "var(--surface)", border: "1px solid var(--border)" }}
            >
              <div className="flex items-center justify-between">
                <span className="text-sm font-semibold" style={{ color: "#7c3aed" }}>
                  📊 {t("stockAnalysis.fullComparisonTitle")}
                </span>
                <button
                  className="text-sm hover:opacity-70"
                  style={{ color: "var(--muted)", border: "none", background: "none", padding: 0, cursor: "pointer" }}
                  onClick={() => setShowInlineComparison(false)}
                >
                  ✕
                </button>
              </div>
              <div className="grid grid-cols-2 gap-1.5 text-sm">
                {/* 公式列 */}
                <div
                  className="rounded px-1.5 py-1 space-y-0.5"
                  style={{ background: "rgba(37,99,235,0.05)", borderLeft: "2px solid #2563eb" }}
                >
                  <div className="flex items-center justify-between">
                    <span className="font-medium" style={{ color: "#2563eb" }}>{t("dualView.decision.formula")}</span>
                    {decision?.action
                      ? <Tag color={getActionColor(displayAction ?? "")}>{t(getActionTKey(displayAction ?? ""))}</Tag>
                      : <span style={{ color: "var(--muted)" }}>—</span>}
                  </div>
                  <div className="font-mono flex gap-2" style={{ color: "var(--color-text-secondary)" }}>
                    <span>
                      {t("stockAnalysis.confidence")} <b>{confidencePct}%</b>
                    </span>
                    <span>|</span>
                    <span>
                      {t("stockAnalysis.position")} <b>{decision ? `${decision.positionPct}%` : "—"}</b>
                    </span>
                  </div>
                  {decision?.reasoning && (
                    <div className="line-clamp-2" style={{ color: "var(--muted)", fontSize: "12px" }}>
                      {decision.reasoning.slice(0, 120)}
                      {decision.reasoning.length > 120 ? "…" : ""}
                    </div>
                  )}
                </div>
                {/* LLM 列 */}
                <div
                  className="rounded px-1.5 py-1 space-y-0.5"
                  style={{ background: "rgba(124,58,237,0.05)", borderLeft: "2px solid #7c3aed" }}
                >
                  <div className="flex items-center justify-between">
                    <span className="font-medium" style={{ color: "#7c3aed" }}>LLM</span>
                    {llmStance
                      ? <Tag color={getActionColor(llmStance)}>{t(getActionTKey(llmStance))}</Tag>
                      : <span style={{ color: "var(--muted)" }}>—</span>}
                  </div>
                  <div className="font-mono flex gap-2" style={{ color: "var(--color-text-secondary)" }}>
                    <span>
                      {t("stockAnalysis.confidence")}{" "}
                      <b>
                        {(() => {
                          const c = extractLlmField(llmDecisionJson, "confidence") as number | null;
                          return c != null ? `${Math.round(c)}%` : "—";
                        })()}
                      </b>
                    </span>
                    <span>|</span>
                    <span>
                      {t("stockAnalysis.position")}{" "}
                      <b>
                        {(() => {
                          const p = extractLlmField(llmDecisionJson, "positionPct") as number | null;
                          return p != null ? `${Math.round(p)}%` : "—";
                        })()}
                      </b>
                    </span>
                  </div>
                  {/* V65: LLM 风险等级 + 止损/止盈百分比 */}
                  {llmFields && (
                    <div
                      className="font-mono flex flex-wrap gap-2"
                      style={{ color: "var(--color-text-secondary)", fontSize: "12px" }}
                    >
                      {llmFields.risk && (
                        <span>
                          {t("stockAnalysis.riskLevel")}{" "}
                          <b style={{ color: getRiskColor(llmFields.risk) }}>
                            {t(getRiskTKey(llmFields.risk))}
                          </b>
                        </span>
                      )}
                      {llmFields.stopLossPct != null && (
                        <span>
                          {t("stockAnalysis.stopLoss")}{" "}
                          <b style={{ color: "var(--sa-red)" }}>{llmFields.stopLossPct.toFixed(1)}%</b>
                        </span>
                      )}
                      {llmFields.takeProfitPct != null && (
                        <span>
                          {t("stockAnalysis.takeProfit")}{" "}
                          <b style={{ color: "var(--sa-green)" }}>{llmFields.takeProfitPct.toFixed(1)}%</b>
                        </span>
                      )}
                    </div>
                  )}
                  {/* V65: LLM 数据缺口 */}
                  {llmFields?.gaps && llmFields.gaps.length > 0 && (
                    <div className="flex flex-wrap gap-1" style={{ fontSize: "11px" }}>
                      <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.gapsLabel")}</span>
                      {llmFields.gaps.slice(0, 3).map((g, i) => (
                        <span
                          key={i}
                          className="px-1 rounded"
                          style={{ background: "rgba(239,68,68,0.10)", color: "#ef4444" }}
                        >
                          {g}
                        </span>
                      ))}
                      {llmFields.gaps.length > 3 && (
                        <span style={{ color: "var(--muted)" }}>
                          {t("stockAnalysis.moreGaps", { count: llmFields.gaps.length - 3 })}
                        </span>
                      )}
                    </div>
                  )}
                  {/* V65: LLM 引用上游论据 */}
                  {llmFields?.evidence && llmFields.evidence.length > 0 && (
                    <div className="space-y-0.5" style={{ fontSize: "11px" }}>
                      <span style={{ color: "var(--muted)" }}>
                        {t("stockAnalysis.evidenceLabel", { count: llmFields.evidence.length })}
                      </span>
                      {llmFields.evidence.slice(0, 2).map((e, i) => (
                        <div key={i} style={{ color: "var(--color-text-secondary)" }}>
                          <span style={{ color: "#7c3aed" }}>[{e.source ?? "?"}]</span>{" "}
                          <span className="line-clamp-1">{e.point ?? ""}</span>
                        </div>
                      ))}
                      {llmFields.evidence.length > 2 && (
                        <div style={{ color: "var(--muted)" }}>
                          {t("stockAnalysis.moreEvidence", { count: llmFields.evidence.length - 2 })}
                        </div>
                      )}
                    </div>
                  )}
                  {llmSummary && (
                    <div className="line-clamp-2" style={{ color: "var(--muted)", fontSize: "12px" }}>
                      {llmSummary.slice(0, 120)}
                      {llmSummary.length > 120 ? "…" : ""}
                    </div>
                  )}
                </div>
              </div>
              {/* V65: 6 维度分歧诊断（带原始分展示） */}
              {decision?.agreementBreakdown && decisionAgreementScore < 80 && (
                <div
                  className="pt-0.5 space-y-0.5"
                  style={{ borderTop: "1px solid var(--border)" }}
                >
                  <div className="flex flex-wrap gap-2 text-sm" style={{ color: "var(--muted)" }}>
                    <span>
                      {decision.agreementBreakdown.actionNote === "opposite"
                        ? t("stockAnalysis.decision.oppositeDirection")
                        : t("stockAnalysis.decision.disagreement")}
                      ({decision.agreementBreakdown.formulaAction} vs {decision.agreementBreakdown.llmAction})
                    </span>
                    {decision.agreementBreakdown.positionGap != null && (
                      <span>
                        {t("stockAnalysis.decision.positionGap", {
                          gap: Math.round(decision.agreementBreakdown.positionGap),
                        })}
                      </span>
                    )}
                    {decision.agreementBreakdown.confidenceGap != null && (
                      <span>
                        {t("stockAnalysis.decision.confidenceGap", {
                          gap: Math.round(decision.agreementBreakdown.confidenceGap),
                        })}
                      </span>
                    )}
                  </div>
                  {/* 原始分柱状条：5 维度（2026-09-21 移除 gaps —— 跨口径 Jaccard 恒 0） */}
                  <div className="grid grid-cols-5 gap-1 text-[11px] font-mono">
                    {([
                      { label: "act", score: decision.agreementBreakdown.actionScore, max: 30 },
                      { label: "pos", score: decision.agreementBreakdown.positionScore, max: 20 },
                      { label: "conf", score: decision.agreementBreakdown.confidenceScore, max: 15 },
                      { label: "risk", score: decision.agreementBreakdown.riskLevelScore, max: 15 },
                      { label: "evid", score: decision.agreementBreakdown.evidenceScore, max: 10 },
                    ] as const).map((dim) => {
                      const s = typeof dim.score === "number" ? dim.score : 0;
                      const ratio = dim.max > 0 ? s / dim.max : 0;
                      const color = ratio >= 0.8
                        ? "#10b981"
                        : ratio >= 0.5
                        ? "#f59e0b"
                        : "#ef4444";
                      return (
                        <div key={dim.label} className="flex flex-col items-center gap-0.5">
                          <span style={{ color: "var(--muted)" }}>{dim.label}</span>
                          <div
                            className="w-full rounded-sm overflow-hidden"
                            style={{ height: 4, background: "var(--surface)" }}
                          >
                            <div
                              style={{
                                width: `${ratio * 100}%`,
                                height: "100%",
                                background: color,
                              }}
                            />
                          </div>
                          <span style={{ color }}>
                            {Math.round(s)}/{dim.max}
                          </span>
                        </div>
                      );
                    })}
                  </div>
                  {/* V65: 风险等级对比 */}
                  {decision.agreementBreakdown.formulaRiskLevel
                    && decision.agreementBreakdown.llmRiskLevel
                    && decision.agreementBreakdown.formulaRiskLevel !== "?"
                    && decision.agreementBreakdown.llmRiskLevel !== "?" && (
                    <div className="text-[11px]" style={{ color: "var(--muted)" }}>
                      {t("stockAnalysis.riskLevelFormula")}
                      <span style={{ color: getRiskColor(decision.agreementBreakdown.formulaRiskLevel) }}>
                        {t(getRiskTKey(decision.agreementBreakdown.formulaRiskLevel))}
                      </span>{" "}
                      vs LLM{" "}
                      <span style={{ color: getRiskColor(decision.agreementBreakdown.llmRiskLevel) }}>
                        {t(getRiskTKey(decision.agreementBreakdown.llmRiskLevel))}
                      </span>
                      {typeof decision.agreementBreakdown.evidenceCount === "number" && (
                        <span className="ml-2">
                          {t("stockAnalysis.llmEvidenceCount", { count: decision.agreementBreakdown.evidenceCount })}
                        </span>
                      )}
                    </div>
                  )}
                </div>
              )}
              {/* 跨系统互证：趋势智选推荐 vs 工作流决策 分歧报告 */}
              {decision?.crossCheck && decision.crossCheck.divergent && (
                <div
                  className="pt-0.5 space-y-1"
                  style={{ borderTop: "1px solid var(--border)" }}
                >
                  <div className="text-[11px]" style={{ color: "#f59e0b" }}>
                    🔀 {t("stockAnalysis.crossCheck.title")}
                  </div>
                  {decision.crossCheck.recoGeneratedAt && decision.crossCheck.decisionGeneratedAt && (
                    <div className="text-[11px]" style={{ color: "var(--muted)" }}>
                      {(() => {
                        const recoMs = new Date(decision.crossCheck.recoGeneratedAt).getTime();
                        const decMs = new Date(decision.crossCheck.decisionGeneratedAt).getTime();
                        const gapDays = Number.isFinite(recoMs) && Number.isFinite(decMs)
                          ? Math.abs(Math.round((decMs - recoMs) / 86400000))
                          : null;
                        return t("stockAnalysis.crossCheck.timeBasis", {
                          recoDate: decision.crossCheck.recoGeneratedAt.slice(0, 10),
                          decisionDate: decision.crossCheck.decisionGeneratedAt.slice(0, 10),
                          gapDays: gapDays ?? "?",
                        });
                      })()}
                    </div>
                  )}
                  <div className="text-[11px]" style={{ color: "var(--color-text-secondary)" }}>
                    {t("stockAnalysis.crossCheck.recoSays", {
                      score: Math.round(decision.crossCheck.recoConfidence),
                      position: decision.crossCheck.recoPositionPct,
                      strategy: decision.crossCheck.recoStrategyType,
                    })}
                    {decision.crossCheck.attentionHeat
                      && ` · ${
                        t("stockAnalysis.crossCheck.attentionHeat", {
                          heat: decision.crossCheck.attentionHeat,
                        })
                      }`}
                  </div>
                  <div className="text-[11px]" style={{ color: "var(--color-text-secondary)" }}>
                    {t("stockAnalysis.crossCheck.workflowSays", {
                      action: decision.crossCheck.decisionAction,
                      position: decision.crossCheck.decisionPositionPct,
                    })}
                  </div>
                  <div className="text-[11px]" style={{ color: "var(--muted)" }}>
                    {t("stockAnalysis.crossCheck.divergenceNote", {
                      recoDays: decision.crossCheck.recoHoldingDays,
                      decisionDays: decision.expectedHoldingDays ?? 7,
                    })}
                  </div>
                  {decision.crossCheck.catalysts.length > 0 && (
                    <div className="space-y-0.5">
                      {decision.crossCheck.catalysts.slice(0, 2).map((cat, i) => (
                        <div
                          key={i}
                          className="line-clamp-1 text-[11px]"
                          style={{ color: "var(--muted)" }}
                        >
                          <span style={{ color: "#10b981" }}>
                            {t("stockAnalysis.crossCheck.catalystLabel", {
                              confidence: Math.round(cat.confidence),
                            })}
                          </span>{" "}
                          {cat.description}
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              )}
            </div>
          )}
        </>
      )}

      {/* 完整详情 Modal（嵌入模式和独立模式共用） */}
      <Modal
        title={
          <div className="flex items-center gap-2">
            <span>{t("stockAnalysis.finalDecision")}</span>
            <Tag color={getActionColor(displayAction ?? "")}>
              {actionLabel(displayAction ?? "")}
            </Tag>
            {asOfDate && (
              <Tag color="purple" title={t("timeTravel.badge.replayTooltip", { date: asOfDate })}>
                ⏪ {t("timeTravel.pageAnchor.untilDate", { date: asOfDate })}
              </Tag>
            )}
          </div>
        }
        open={expanded}
        onCancel={() => setExpanded(false)}
        footer={null}
        width="80vw"
        style={{ top: 20 }}
        styles={{ body: { maxHeight: "80vh", overflow: "auto" } }}
      >
        <div className="mb-4">
          <div className="flex justify-between items-center mb-1">
            <div className="flex items-center gap-2">
              <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.confidence")}</span>
              <span
                className="font-mono font-semibold"
                style={{ color: meterColor, fontSize: 22 }}
              >
                {confidencePct}%
              </span>
              {/* V68 修复: 移除 adjustedConfidence 显示 */}
              <span
                className="text-sm px-2 py-0.5 rounded font-medium"
                style={{
                  background: `${confidenceLabelColor}18`,
                  color: confidenceLabelColor,
                  border: `1px solid ${confidenceLabelColor}40`,
                }}
              >
                {confidenceLabel}
              </span>
            </div>
            {decisionAgreementScore !== null && (
              <span
                className="text-sm"
                style={{ color: "var(--muted)" }}
              >
                📊 {t("stockAnalysis.consensusAbbr")} {decisionAgreementScore}/100
              </span>
            )}
          </div>
          <div
            className="relative"
            style={{ height: 14, borderRadius: 7, background: "var(--surface)", overflow: "hidden" }}
          >
            <div
              style={{
                width: `${confidencePct}%`,
                height: "100%",
                borderRadius: 7,
                background: `linear-gradient(to right, ${meterColor}88, ${meterColor})`,
                transition: "width 0.6s ease",
              }}
            />
          </div>
        </div>

        <div
          className="grid gap-3 mb-4"
          style={{ gridTemplateColumns: "repeat(5, 1fr)" }}
        >
          {decision.targetPrice && (
            <div className="text-center p-3 rounded" style={{ background: "var(--surface)" }}>
              <div className="text-sm" style={{ color: "var(--muted)" }}>{t("stockAnalysis.targetPrice")}</div>
              <div className="text-lg font-semibold font-mono">¥{decision.targetPrice}</div>
            </div>
          )}
          {decision.stopLoss && (
            <div className="text-center p-3 rounded" style={{ background: "var(--surface)" }}>
              <div className="text-sm" style={{ color: "var(--muted)" }}>{t("stockAnalysis.stopLoss")}</div>
              <div className="text-lg font-semibold font-mono" style={{ color: "var(--sa-red)" }}>
                ¥{decision.stopLoss}
              </div>
            </div>
          )}
          <div className="text-center p-3 rounded" style={{ background: "var(--surface)" }}>
            <div className="text-sm" style={{ color: "var(--muted)" }}>{t("stockAnalysis.position")}</div>
            <div className="text-lg font-semibold font-mono">{decision.positionPct}%</div>
          </div>
          {upside != null && (
            <div className="text-center p-3 rounded" style={{ background: "var(--surface)" }}>
              <div className="text-sm" style={{ color: "var(--muted)" }}>{t("stockAnalysis.expectedUpside")}</div>
              <div
                className="text-lg font-semibold font-mono"
                style={{ color: upside >= 0 ? "var(--sa-green)" : "var(--sa-red)" }}
              >
                {upside >= 0 ? "+" : ""}
                {upside.toFixed(1)}%
              </div>
            </div>
          )}
          <div className="text-center p-3 rounded" style={{ background: "var(--surface)" }}>
            <div className="text-sm" style={{ color: "var(--muted)" }}>{t("stockAnalysis.riskLevel")}</div>
            <div
              className="text-lg font-semibold"
              style={{
                color: getRiskColor(decision.riskLevel),
              }}
            >
              {t(getRiskTKey(decision.riskLevel))}
            </div>
          </div>
        </div>

        <div
          className="sa-markdown-content text-sm mb-4 p-4 rounded"
          style={{ background: "var(--surface)" }}
        >
          {cleanToolCallTags(decision.reasoning || "")
            ? <ReportMarkdown content={cleanToolCallTags(decision.reasoning || "")} isDark={isDark} />
            : <span style={{ color: "var(--muted)" }}>{t("stockAnalysis.noDecisionReasoning")}</span>}
        </div>

        {/* 数据质量诊断面板：展示决策需要的数据 / 上游给出的数据 / 数据差距 */}
        {dataQualityReport
          ? (
            <div
              className="mb-4 p-3 rounded"
              style={{
                background: dataQualityReport.grade === "F" || dataQualityReport.grade === "D"
                  ? "rgba(239, 68, 68, 0.06)"
                  : "rgba(16, 185, 129, 0.05)",
                borderLeft: `3px solid ${
                  dataQualityReport.grade === "F" || dataQualityReport.grade === "D" ? "#ef4444" : "#10b981"
                }`,
              }}
            >
              <div className="flex items-center justify-between mb-2">
                <div className="flex items-center gap-2">
                  <span className="text-sm font-semibold">
                    {t("stockAnalysis.dqDiagnostics")}
                  </span>
                  <Tag
                    color={dataQualityReport.grade === "A" || dataQualityReport.grade === "B"
                      ? "success"
                      : dataQualityReport.grade === "C"
                      ? "warning"
                      : "error"}
                    style={{ fontSize: 12, lineHeight: "20px", height: 20, paddingInline: 6 }}
                  >
                    {dataQualityReport.grade} · {dataQualityReport.score.toFixed(1)}
                  </Tag>
                </div>
                <span className="text-xs font-mono" style={{ color: "var(--muted)" }}>
                  {t("stockAnalysis.dqHighConfCount", {
                    good: dataQualityReport.good_count,
                    total: dataQualityReport.total_analysts,
                    avg: dataQualityReport.avg_confidence.toFixed(1),
                  })}
                </span>
              </div>
              <div className="text-sm mb-2" style={{ color: "var(--color-text-secondary)" }}>
                {dataQualityReport.summary}
              </div>
              {/* 因子完整度面板 */}
              {dataQualityReport.factor_completeness_pct !== undefined && (
                <div className="mb-3 p-2 rounded" style={{ background: "var(--surface)" }}>
                  <div className="flex items-center justify-between mb-2">
                    <span className="text-xs font-semibold">{t("stockAnalysis.factorCompleteness")}</span>
                    <span
                      className="text-xs font-mono"
                      style={{
                        color: dataQualityReport.factor_completeness_pct >= 70
                          ? "#10b981"
                          : dataQualityReport.factor_completeness_pct >= 40
                          ? "#f59e0b"
                          : "#ef4444",
                      }}
                    >
                      {dataQualityReport.factor_completeness_pct.toFixed(1)}%
                    </span>
                  </div>
                  {dataQualityReport.missing_factors && dataQualityReport.missing_factors.length > 0 && (
                    <div className="text-xs" style={{ color: "var(--color-text-secondary)" }}>
                      <span className="font-medium">{t("stockAnalysis.missingFactors")}</span>
                      <span className="ml-1" style={{ color: "#ef4444" }}>
                        {dataQualityReport.missing_factors.join("、")}
                      </span>
                    </div>
                  )}
                  {(!dataQualityReport.missing_factors || dataQualityReport.missing_factors.length === 0) && (
                    <div className="text-xs" style={{ color: "#10b981" }}>
                      {t("stockAnalysis.allFactorsComplete")}
                    </div>
                  )}
                  {
                    /*
                    2026-09-21: 上游取数缺口 —— 与「缺失因子」**分列**显示。
                    语义差别：缺失因子 = 本节点要消费的因子没值（进 factor_completeness 分母）；
                    上游缺口 = **上游节点没取到数**（我方采集缺陷，不参与评分）。
                    故用橙色（提示）而非红色（问题），并在文案里已写明「仅提示，不影响评分」。
                    ⚠️ 不要把它并进上面的 missing_factors 列表：那会让列表长度与
                    `pm_compute_factor_completeness` 的分母口径不一致（Rhai 侧有注释守住）。
                  */
                  }
                  {dataQualityReport.upstream_data_gaps
                    && dataQualityReport.upstream_data_gaps.length > 0 && (
                    <div className="text-xs" style={{ color: "var(--color-text-secondary)" }}>
                      <span className="font-medium">{t("stockAnalysis.upstreamDataGaps")}</span>
                      {dataQualityReport.upstream_data_gaps.map((g) => (
                        <div key={g} className="ml-1" style={{ color: "#f59e0b" }}>
                          {g}
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              )}
              <Collapse
                ghost
                defaultActiveKey={dataQualityReport.grade === "F" || dataQualityReport.grade === "D" ? ["diag"] : []}
                items={[{
                  key: "diag",
                  label: (
                    <span className="text-sm" style={{ color: "var(--muted)" }}>
                      {t("stockAnalysis.analystDataGapDetails")}
                    </span>
                  ),
                  children: (
                    <div className="overflow-x-auto">
                      <table className="text-xs w-full" style={{ borderCollapse: "collapse" }}>
                        <thead>
                          <tr style={{ borderBottom: "1px solid var(--border)" }}>
                            <th className="text-left py-1.5 px-2" style={{ color: "var(--muted)" }}>
                              {t("stockAnalysis.dqTableAnalyst")}
                            </th>
                            <th className="text-left py-1.5 px-2" style={{ color: "var(--muted)" }}>
                              {t("stockAnalysis.dqTableExpectedData")}
                            </th>
                            <th className="text-right py-1.5 px-2" style={{ color: "var(--muted)" }}>
                              {t("stockAnalysis.dqTableConfidence")}
                            </th>
                            <th className="text-right py-1.5 px-2" style={{ color: "var(--muted)" }}>
                              {t("stockAnalysis.dqTablePlaceholder")}
                            </th>
                            {/* 2026-09-21 新增列：本节点报告质量分（事实量，非等级；取值见下方 rq 注释）。 */}
                            <th className="text-right py-1.5 px-2" style={{ color: "var(--muted)" }}>
                              {t("stockAnalysis.dqTableReportQuality")}
                            </th>
                            <th className="text-left py-1.5 px-2" style={{ color: "var(--muted)" }}>
                              {t("stockAnalysis.dqTableStatus")}
                            </th>
                            <th className="text-left py-1.5 px-2" style={{ color: "var(--muted)" }}>
                              {t("stockAnalysis.dqTableGapReason")}
                            </th>
                          </tr>
                        </thead>
                        <tbody>
                          {Object.entries(dataQualityReport.diagnostics).map(([key, diag]) => {
                            // 2026-09-12: "untrusted"（strict_mode 降级）此前落入 else 分支，
                            // 被渲染成「✅ 正常」——与后端语义相反。i18n key dqStatusUntrusted
                            // 早已存在于全部 11 种语言，但从未被消费。
                            const statusColor = diag.status === "missing"
                              ? "#ef4444"
                              : diag.status === "low" || diag.status === "untrusted"
                              ? "#f59e0b"
                              : "#10b981";
                            const statusLabel = diag.status === "missing"
                              ? t("stockAnalysis.dqStatusMissing")
                              : diag.status === "low"
                              ? t("stockAnalysis.dqStatusLowConfidence")
                              : diag.status === "untrusted"
                              ? t("stockAnalysis.dqStatusUntrusted")
                              : t("stockAnalysis.dqStatusNormal");
                            const confText = diag.confidence < 0
                              ? "—"
                              : `${diag.confidence.toFixed(0)}`;
                            // 2026-09-12: 报告文本失败标记数（客观证据，独立于自评 confidence）
                            const phHits = diag.placeholder_hits ?? 0;
                            // 2026-09-21: 本节点**自己的**报告质量分（0-100）。
                            //   与 `AnalystDataQualityModal` 的「本节点报告质量」同源同口径
                            //   （`data-quality.rhai::diag_for` 的 `report_quality`）；本表每节点一行
                            //   ⇒ 各行值互不相同，正好与顶部那个**全局聚合**值区分开。
                            //   undefined = 旧版快照缺该字段 ⇒ 显示 "—" 并给 tooltip，**不能当 0**
                            //   （0 是合法取值：该节点未注入报告）。
                            //   档位来自 `reportQualitySeverity` 单一来源，勿在此重写阈值。
                            const rq = diag.report_quality;
                            const rqSeverity = reportQualitySeverity(rq);
                            const rqColor = rqSeverity === null
                              ? "var(--muted)"
                              : rqSeverity === "good"
                              ? "#10b981"
                              : rqSeverity === "warning"
                              ? "#f59e0b"
                              : "#ef4444";
                            return (
                              <tr
                                key={key}
                                style={{ borderBottom: "1px solid var(--border)" }}
                              >
                                <td className="py-1.5 px-2 font-medium">{diag.name}</td>
                                <td className="py-1.5 px-2" style={{ color: "var(--color-text-secondary)" }}>
                                  {diag.expected_data}
                                </td>
                                <td
                                  className="py-1.5 px-2 text-right font-mono"
                                  style={{ color: diag.confidence < 0 ? "var(--muted)" : statusColor }}
                                >
                                  {confText}
                                </td>
                                <td
                                  className="py-1.5 px-2 text-right font-mono"
                                  style={{ color: phHits > 0 ? "#f59e0b" : "var(--muted)" }}
                                  title={phHits > 0
                                    ? t("stockAnalysis.dqPlaceholderTooltip", { count: phHits })
                                    : undefined}
                                >
                                  {phHits > 0 ? `${phHits}` : "—"}
                                </td>
                                <td
                                  className="py-1.5 px-2 text-right font-mono"
                                  style={{ color: rqColor }}
                                  title={rq === undefined
                                    ? t("stockAnalysis.analystReport.dqNodeReportQualityUnavailable")
                                    : undefined}
                                >
                                  {rq === undefined ? "—" : rq.toFixed(0)}
                                </td>
                                <td className="py-1.5 px-2" style={{ color: statusColor }}>
                                  {statusLabel}
                                </td>
                                <td className="py-1.5 px-2" style={{ color: "var(--color-text-secondary)" }}>
                                  {diag.gap_reason || "—"}
                                </td>
                              </tr>
                            );
                          })}
                        </tbody>
                      </table>
                    </div>
                  ),
                }]}
              />
            </div>
          )
          : (
            // dataQualityReport 为 null（解析失败/旧版输出）时显示原始数据片段
            // 方便用户/开发者一眼看出是"空"还是"格式不匹配"还是"字段缺失"
            <div
              className="mb-4 p-2 rounded text-xs"
              style={{ background: "var(--surface)", borderLeft: "3px solid var(--sa-amber, #f59e0b)" }}
            >
              <div className="font-semibold mb-1" style={{ color: "#f59e0b" }}>
                {t("stockAnalysis.dataQuality.renderFailed")}
              </div>
              <div style={{ color: "var(--muted)" }}>
                {t("stockAnalysis.dataQuality.summaryLength")}
                <span className="font-mono ml-1">{dataQualitySummary.length}</span>
                {dataQualitySummary.length > 0 && (
                  <>
                    <span className="ml-2">{t("stockAnalysis.dataQuality.first200Chars")}</span>
                    <pre
                      className="mt-1 p-2 rounded font-mono"
                      style={{ background: "var(--background)", whiteSpace: "pre-wrap", wordBreak: "break-all" }}
                    >
                      {dataQualitySummary.slice(0, 200)}
                      {dataQualitySummary.length > 200 ? "..." : ""}
                    </pre>
                  </>
                )}
                {staleRecordSummary && (
                  // V66 修复(2026-07-29): snapshot 为空的旧版记录，显示重跑按钮
                  <div className="mt-2 flex items-center gap-2 flex-wrap">
                    <span>{staleRecordSummary}</span>
                    {analysisId && (
                      <Tooltip title={t("stockAnalysis.rerunDecisionHint")}>
                        <Button
                          size="small"
                          type="primary"
                          onClick={() => useStockAnalysisStore.getState().rerunDecision(analysisId)}
                        >
                          {t("stockAnalysis.rerunDecision")}
                        </Button>
                      </Tooltip>
                    )}
                  </div>
                )}
                {dataQualitySummary.length === 0 && !staleRecordSummary && (
                  <div className="mt-1">{t("stockAnalysis.dataQuality.emptySummaryHint")}</div>
                )}
              </div>
            </div>
          )}

        {/* 决策输入诊断面板：展示 portfolio-mgr 实际消费的 16 个上游节点的数据符合度 */}
        {
          /* 与上方"数据质量诊断"互补：那个只看 10 个分析师的 LLM 输出质量（data-quality 节点视角），
            这个看所有决策输入节点（含辩手/trader/风控/算法工具等）的数据完整性（portfolio-mgr 视角） */
        }
        {decisionInputsReport.length > 0 && <DecisionInputsDiagPanel report={decisionInputsReport} />}

        {/* [Phase 2 step 10] Modal 内分歧时 LLM 对比标注 */}
        {decisionAgreementScore !== null && decisionAgreementScore < disagreementThreshold && llmSummary && (
          <div
            className="text-sm mb-4 p-3 rounded"
            style={{ background: "rgba(124, 58, 237, 0.08)", borderLeft: "3px solid #7c3aed" }}
          >
            <span className="font-medium" style={{ color: "#7c3aed" }}>
              💡 {t("stockAnalysis.llmPerspective")}:
            </span>
            <span className="ml-1" style={{ color: "var(--color-text-secondary)" }}>
              {llmSummary}
            </span>
          </div>
        )}

        <div className="flex gap-2 items-center flex-wrap">
          {stockCode && !watchlisted && (
            <Button type="dashed" loading={adding} onClick={addToWatchlist}>
              ⭐ {t("stockAnalysis.addToWatchlist")}
            </Button>
          )}
          {watchlisted && <Tag color="gold">⭐ {t("stockAnalysis.inWatchlist")}</Tag>}
          {stockCode && (
            <>
              <Button
                icon={<ReloadOutlined />}
                onClick={() => {
                  useStockAnalysisStore.getState().startAnalysis(
                    stockCode,
                    analysisId ? { parentAnalysisId: analysisId } : undefined,
                  );
                }}
              >
                {t("stockAnalysis.reAnalyze")}
              </Button>
              <Button icon={<span>💬</span>} onClick={handleAskAI}>
                {t("stockAnalysis.askAI")}
              </Button>
              <Dropdown
                menu={{
                  items: exportMenuItems.map((item) => ({
                    key: item.key,
                    icon: item.icon,
                    label: item.label,
                    disabled: exporting === item.key,
                    onClick: () => handleExport(item.key),
                  })),
                }}
                trigger={["click"]}
              >
                <Button loading={exporting !== null} icon={<span>📥</span>}>
                  {t("stockAnalysis.exportReport")}
                </Button>
              </Dropdown>
            </>
          )}
        </div>
      </Modal>
    </>
  );
}

// ── 决策输入诊断面板：展示 portfolio-mgr 16 个上游节点的数据符合度 ──
// 纯前端展示组件，数据来自 store.decisionInputsReport（不持久化）
// 按 factor 分组渲染，颜色标注 missing/low/untrusted/normal 四种状态
function DecisionInputsDiagPanel({ report }: { report: DecisionInputsReport }) {
  const { t } = useTranslation();
  const summary = useMemo(() => summarizeDecisionInputs(report), [report]);
  // 按 factor 分组
  const grouped = useMemo(() => {
    const m = new Map<string, typeof report>();
    for (const item of report) {
      const arr = m.get(item.factor) ?? [];
      arr.push(item);
      m.set(item.factor, arr);
    }
    return Array.from(m.entries());
  }, [report]);

  const hasIssue = summary.missing > 0 || summary.low > 0 || summary.untrusted > 0;
  const bgColor = hasIssue ? "rgba(239, 68, 68, 0.04)" : "rgba(16, 185, 129, 0.04)";
  const borderColor = hasIssue ? "#ef4444" : "#10b981";

  return (
    <div
      className="mb-4 p-3 rounded"
      style={{ background: bgColor, borderLeft: `3px solid ${borderColor}` }}
    >
      <div className="flex items-center justify-between mb-2">
        <div className="flex items-center gap-2">
          <span className="text-sm font-semibold">{t("stockAnalysis.decisionInputsDiag")}</span>
          <Tag
            color={hasIssue ? "error" : "success"}
            style={{ fontSize: 12, lineHeight: "20px", height: 20, paddingInline: 6 }}
          >
            {t("stockAnalysis.diNormalCount", { normal: summary.normal, total: summary.total })}
          </Tag>
        </div>
        <span className="text-xs font-mono" style={{ color: "var(--muted)" }}>
          {summary.missing > 0 && (
            <span style={{ color: "#ef4444" }}>{t("stockAnalysis.diMissing", { count: summary.missing })} ·</span>
          )}
          {summary.low > 0 && (
            <span style={{ color: "#f59e0b" }}>{t("stockAnalysis.diLowConfidence", { count: summary.low })} ·</span>
          )}
          {summary.untrusted > 0 && (
            <span style={{ color: "#ef4444" }}>{t("stockAnalysis.diUntrusted", { count: summary.untrusted })} ·</span>
          )}
          <span>{t("stockAnalysis.diTotal", { total: summary.total })}</span>
        </span>
      </div>
      <Collapse
        ghost
        defaultActiveKey={hasIssue ? ["inputs-diag"] : []}
        items={[{
          key: "inputs-diag",
          label: <span className="text-sm" style={{ color: "var(--muted)" }}>{t("stockAnalysis.groupByFactor")}</span>,
          children: (
            <div className="overflow-x-auto">
              <table className="text-xs w-full" style={{ borderCollapse: "collapse" }}>
                <thead>
                  <tr style={{ borderBottom: "1px solid var(--border)" }}>
                    <th className="text-left py-1.5 px-2" style={{ color: "var(--muted)" }}>
                      {t("stockAnalysis.diTableFactor")}
                    </th>
                    <th className="text-left py-1.5 px-2" style={{ color: "var(--muted)" }}>
                      {t("stockAnalysis.diTableNode")}
                    </th>
                    <th className="text-left py-1.5 px-2" style={{ color: "var(--muted)" }}>
                      {t("stockAnalysis.diTableRole")}
                    </th>
                    <th className="text-right py-1.5 px-2" style={{ color: "var(--muted)" }}>
                      {t("stockAnalysis.diTableWeight")}
                    </th>
                    <th className="text-right py-1.5 px-2" style={{ color: "var(--muted)" }}>
                      {t("stockAnalysis.diTableConfidence")}
                    </th>
                    <th className="text-left py-1.5 px-2" style={{ color: "var(--muted)" }}>
                      {t("stockAnalysis.diTableDirection")}
                    </th>
                    <th className="text-left py-1.5 px-2" style={{ color: "var(--muted)" }}>
                      {t("stockAnalysis.diTableStatus")}
                    </th>
                    <th className="text-left py-1.5 px-2" style={{ color: "var(--muted)" }}>
                      {t("stockAnalysis.diTableNote")}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {grouped.map(([factor, items]) => (
                    items.map((item, idx) => {
                      const statusColor = item.status === "missing" || item.status === "untrusted"
                        ? "#ef4444"
                        : item.status === "low"
                        ? "#f59e0b"
                        : "#10b981";
                      const statusLabel = item.status === "missing"
                        ? t("stockAnalysis.dqStatusMissing")
                        : item.status === "untrusted"
                        ? t("stockAnalysis.dqStatusUntrusted")
                        : item.status === "low"
                        ? t("stockAnalysis.dqStatusLowConfidence")
                        : t("stockAnalysis.dqStatusNormal");
                      const confText = item.confidence === null
                        ? "—"
                        : `${item.confidence.toFixed(0)}`;
                      const weightText = item.weight === null
                        ? "—"
                        : `${(item.weight * 100).toFixed(0)}%`;
                      return (
                        <tr
                          key={`${item.nodeId}-${idx}`}
                          style={{
                            borderBottom: idx === items.length - 1
                              ? "2px solid var(--border)"
                              : "1px solid var(--border)",
                          }}
                        >
                          {idx === 0 && (
                            <td
                              className="py-1.5 px-2 font-medium align-top"
                              rowSpan={items.length}
                              style={{ color: "var(--color-text-secondary)" }}
                            >
                              {factor}
                            </td>
                          )}
                          <td className="py-1.5 px-2 font-mono" style={{ color: "var(--muted)" }}>
                            {item.nodeId}
                          </td>
                          <td className="py-1.5 px-2">{item.role}</td>
                          <td className="py-1.5 px-2 text-right font-mono" style={{ color: "var(--muted)" }}>
                            {weightText}
                          </td>
                          <td
                            className="py-1.5 px-2 text-right font-mono"
                            style={{ color: item.confidence === null ? "var(--muted)" : statusColor }}
                          >
                            {confText}
                          </td>
                          <td className="py-1.5 px-2" style={{ color: "var(--color-text-secondary)" }}>
                            {item.stance || "—"}
                          </td>
                          <td className="py-1.5 px-2" style={{ color: statusColor }}>
                            {statusLabel}
                          </td>
                          <td className="py-1.5 px-2" style={{ color: "var(--color-text-secondary)" }}>
                            {item.note || "—"}
                          </td>
                        </tr>
                      );
                    })
                  ))}
                </tbody>
              </table>
            </div>
          ),
        }]}
      />
    </div>
  );
}