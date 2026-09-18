// i18n-exempt: 业务逻辑/API 描述/日志字符串，非 UI 展示文本
/**
 * What-If Backtest — 结构化参数方案 Phase 4
 *
 * 核心功能：
 * 1. 选取一条历史分析记录
 * 2. 读取其 blackboard_snapshot 中的结构化 params
 * 3. 允许用户修改任意参数（滑块/下拉）
 * 4. 客户端重新执行 portfolio-mgr 确定性公式（Rhai → TypeScript）
 * 5. 对比修改前后的决策差异
 *
 * 前提：portfolio-mgr 已从 Agent 改为 CodeNode（Rhai 确定性公式）
 */

import { getDefaultVariables } from "@/components/settings/StockAnalysisConfigPanel";
import { invoke } from "@/lib/invoke";
import { Button, Card, Collapse, Empty, InputNumber, Select, SelectProps, Slider, Tag } from "antd";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { ArrowRightOutlined } from "@ant-design/icons";

// ── 类型定义 ──

/** 从后端返回的历史分析记录 */
interface AnalysisRecord {
  id: string;
  stockCode: string;
  stockName: string;
  decisionJson: string | null;
  /** 列表场景不返回，详情页通过 get_stock_analysis 单独获取 */
  blackboardSnapshot?: string | null;
  createdAt: number;
  status: string;
  analysisKind: string;
  asOfDate: string | null;
}

/** portfolio-mgr 的输入参数 */
interface PmInputParams {
  totalScore: number;
  dqiScore: number;
  overallRisk: string;
  catalystLevel: string;
  consensusScore: number;
}

/** portfolio-mgr 的输出决策 */
interface PmDecision {
  decision: string;
  positionPct: number;
  confidence: number;
  riskLevel: string;
  stopLossPct: number;
  takeProfitPct: number;
  reasoning: string;
  /** 决策追溯链（来自 portfolio-mgr.rhai 完整输出） */
  decisionTrail?: Array<{
    ruleId: string;
    status: string;
    detail?: string;
    timestamp?: string;
  }>;
  /** 技术面否决详情（来自 portfolio-mgr.rhai technical_veto） */
  technicalVeto?: {
    vetoed: boolean;
    ruleId?: string;
    reason?: string;
  };
  /** 模拟门信息（S-501~503） */
  simulationGate?: {
    vetoed: boolean;
    ruleId?: string;
    reason?: string;
    preSimAction: string;
    preSimPositionPct: number;
    simStability?: number;
    simLiquidity?: number;
    simImpact?: number;
  };
  /** 模拟门前的原始 action */
  preSimAction?: string;
  /** 模拟门前的原始仓位 */
  preSimPositionPct?: number;
}

// ── 默认值 ──

const DEFAULT_PARAMS: PmInputParams = {
  totalScore: 50,
  dqiScore: 50,
  overallRisk: "中",
  catalystLevel: "无催化剂",
  consensusScore: 50,
};

// ── 前端 Rhai 公式移植（与 portfolio-mgr.rhai 保持一致）──
// 优先调用后端 Rhai 引擎，fallback 到本地 TS 版本

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

async function computeDecisionBackend(
  params: PmInputParams,
  snapshot?: Record<string, unknown> | null,
): Promise<PmDecision | null> {
  try {
    const invokeParams: Record<string, unknown> = {
      totalScore: params.totalScore,
      dqiScore: params.dqiScore,
      overallRisk: params.overallRisk,
      catalystLevel: params.catalystLevel,
      consensusScore: params.consensusScore,
    };
    // 携带 DAG 快照为完整公式提供上游参数
    if (snapshot) {
      invokeParams.blackboardSnapshot = JSON.stringify(snapshot);
    }
    const result = await invoke("compute_what_if", {
      params: invokeParams,
    }) as Record<string, unknown>;
    if (result) {
      return {
        decision: result.decision as string,
        positionPct: Math.round(result.positionPct as number),
        confidence: Math.round(result.confidence as number),
        riskLevel: result.riskLevel as string,
        stopLossPct: result.stopLossPct as number,
        takeProfitPct: result.takeProfitPct as number,
        reasoning: result.reasoning as string,
        // 这些字段当前在 compute_what_if 简化版中不可用，保留以供完整版加载
        decisionTrail: (result as Record<string, unknown>).decisionTrail as PmDecision["decisionTrail"],
        technicalVeto: (result as Record<string, unknown>).technicalVeto as PmDecision["technicalVeto"],
        simulationGate: (result as Record<string, unknown>).simulationGate as PmDecision["simulationGate"],
        preSimAction: (result as Record<string, unknown>).preSimAction as string | undefined,
        preSimPositionPct: (result as Record<string, unknown>).preSimPositionPct as number | undefined,
      };
    }
  } catch (e) {
    console.warn("Backend formula failed, falling back to TS:", e);
  }
  return null;
}

function computeDecisionLocal(params: PmInputParams): PmDecision {
  const { totalScore, dqiScore, overallRisk, catalystLevel, consensusScore } = params;

  // 辩论收敛调整
  const consensusAdj = ((consensusScore - 50) / 100) * 10;

  // 数据质量调整
  const dqiAdj = ((dqiScore - 50) / 100) * 5;

  // 风险调整
  const riskAdjustment = (() => {
    switch (overallRisk) {
      case "低":
        return 5;
      case "高":
        return -5;
      case "极高":
        return -10;
      default:
        return 0;
    }
  })();

  // 催化剂加成
  const catalystBonus = (() => {
    switch (catalystLevel) {
      case "L3估值体系级":
        return 12;
      case "L2业绩拐点级":
        return 6;
      case "L1普通消息":
        return 2;
      case "L-3退市/造假级":
        return -12;
      case "L-2业绩暴雷级":
        return -6;
      case "L-1普通利空":
        return -2;
      default:
        return 0;
    }
  })();

  // 最终置信度（简化版：不含 institutionalTrace，完整公式通过 blackboard 提供）
  const adjustment = consensusAdj + dqiAdj + riskAdjustment + catalystBonus;
  const confidence = clamp(totalScore + adjustment, 0, 100);

  // 仓位推导
  const basePos = riskAdjustment >= 0 ? confidence * 0.8 : confidence * 0.5;
  const positionPct = clamp(basePos, 0, 100);

  // 最终动作
  const decision = (() => {
    if (confidence >= 80 && positionPct >= 30) { return "增持"; }
    if (confidence >= 60) { return "买入"; }
    if (confidence >= 40) { return "持有"; }
    if (positionPct < 10) { return "减持"; }
    return "持有";
  })();

  // riskLevel 判定（简化版：沿用输入的前端显示）
  const riskLevel = overallRisk;

  return {
    decision,
    positionPct: Math.round(positionPct),
    confidence: Math.round(confidence),
    riskLevel,
    stopLossPct: positionPct > 0 ? 8.0 : 0,
    takeProfitPct: positionPct > 0 ? 15.0 : 0,
    reasoning: `确定性公式结果: totalScore=${totalScore.toFixed(0)}, dqi=${
      dqiScore.toFixed(0)
    }, risk=${overallRisk}, catalyst=${catalystLevel}, consensus=${consensusScore}, adjustment=${
      adjustment.toFixed(1)
    }, confidence=${Math.round(confidence)}, position=${Math.round(positionPct)}`,
  };
}

// ── 参数解析 ──

/** 从 blackboard_snapshot 中提取 portfolio-mgr 输入参数
 *
 * Phase 5: 优先从 `params.portfolio-mgr.input_params` 读取（CodeNode 直接保存的
 * 原始 input_mapping 解析值快照），fallback 到从各上游节点 params 重建。
 */
function extractParamsFromSnapshot(snapshot: Record<string, unknown>): PmInputParams {
  const params: PmInputParams = { ...DEFAULT_PARAMS };

  // Phase 5: 优先从 params.portfolio-mgr.input_params 读取
  // 这是 code_executor.rs 直接保存的 input_mapping 解析值快照
  const pmParams = snapshot["params.portfolio-mgr"] as Record<string, unknown> | undefined;
  const inputParams = pmParams?.input_params as Record<string, unknown> | undefined;
  if (inputParams) {
    if (typeof inputParams.totalScore === "number") { params.totalScore = inputParams.totalScore; }
    if (typeof inputParams.dqiScore === "number") { params.dqiScore = inputParams.dqiScore; }
    if (typeof inputParams.overallRisk === "string") { params.overallRisk = inputParams.overallRisk; }
    if (typeof inputParams.catalystLevel === "string") { params.catalystLevel = inputParams.catalystLevel; }
    if (typeof inputParams.consensusScore === "number") { params.consensusScore = inputParams.consensusScore; }
    // institutionalTrace 已整合到完整公式的 blackboard 中，不再作为简化版的显式参数
    return params;
  }

  // Fallback: 从 params.portfolio-mgr result 反推
  if (pmParams && typeof pmParams.totalScore === "number") {
    params.totalScore = pmParams.totalScore;
  }
  if (pmParams && typeof pmParams.dqiScore === "number") {
    params.dqiScore = pmParams.dqiScore;
  }

  // 从 params.data-quality 读取 dqi_score
  const dqParams = snapshot["params.data-quality"] as Record<string, unknown> | undefined;
  if (dqParams && typeof dqParams.score === "number") {
    params.dqiScore = dqParams.score;
  }

  // 从 params.a-catalyst 读取催化剂参数
  const catParams = snapshot["params.a-catalyst"] as Record<string, unknown> | undefined;
  if (catParams) {
    if (catParams.catalyst_level) { params.catalystLevel = String(catParams.catalyst_level); }
  }

  // 尝试从决策 JSON 反推 totalScore
  const decoded = tryParseDecisionJson(snapshot["portfolio-mgr"]) as Record<string, unknown> | undefined;
  if (decoded && typeof decoded.confidence === "number") {
    // 保留用户已有的决策值作为参考，但 params 优先
  }

  return params;
}

function tryParseDecisionJson(input: unknown): unknown {
  if (!input) { return null; }
  // 可能是字符串 JSON，也可能是对象
  if (typeof input === "string") {
    try {
      return JSON.parse(input);
    } catch {
      return null;
    }
  }
  return input;
}

// ── UI 组件 ──

/** 原始决策摘要 */
function originalDecisionSummary(snapshot: Record<string, unknown>): PmDecision | null {
  // portfolio-mgr 的 output
  const pmOutput = snapshot["portfolio-mgr"];
  if (!pmOutput) { return null; }

  const parsed = tryParseDecisionJson(pmOutput) as Record<string, unknown> | undefined;
  if (!parsed || !parsed.decision && !parsed.result) { return null; }

  // CodeNode 的 result 在 output.result 中
  const result = (parsed.result ?? parsed) as Record<string, unknown>;

  // 从 decision_json 解析
  if (typeof result.decision === "string") {
    // 提取 decision_trail（完整 portfolio-mgr 输出常有此字段）
    const rawTrail = result.decision_trail ?? result.decisionTrail;
    const decisionTrail: PmDecision["decisionTrail"] = Array.isArray(rawTrail)
      ? rawTrail.map((t: Record<string, unknown>) => ({
        ruleId: String(t.ruleId ?? t.rule_id ?? ""),
        status: String(t.status ?? ""),
        detail: t.detail as string | undefined,
        timestamp: t.timestamp as string | undefined,
      }))
      : undefined;

    // 提取 technical_veto
    const rawVeto = result.technical_veto ?? result.technicalVeto;
    const technicalVeto: PmDecision["technicalVeto"] = rawVeto && typeof rawVeto === "object"
      ? {
        vetoed: Boolean((rawVeto as Record<string, unknown>).vetoed),
        ruleId: String(
          (rawVeto as Record<string, unknown>).ruleId ?? (rawVeto as Record<string, unknown>).rule_id ?? "",
        ),
        reason: (rawVeto as Record<string, unknown>).reason as string | undefined,
      }
      : undefined;

    // 提取 simulation_gate
    const rawGate = result.simulation_gate ?? result.simulationGate;
    const simulationGate: PmDecision["simulationGate"] = rawGate && typeof rawGate === "object"
      ? {
        vetoed: Boolean((rawGate as Record<string, unknown>).vetoed),
        ruleId: String(
          (rawGate as Record<string, unknown>).ruleId ?? (rawGate as Record<string, unknown>).rule_id ?? "",
        ),
        reason: (rawGate as Record<string, unknown>).reason as string | undefined,
        preSimAction: String(
          (rawGate as Record<string, unknown>).preSimAction ?? (rawGate as Record<string, unknown>).pre_sim_action
            ?? "",
        ),
        preSimPositionPct: Number(
          (rawGate as Record<string, unknown>).preSimPositionPct
            ?? (rawGate as Record<string, unknown>).pre_sim_position_pct ?? 0,
        ),
        simStability: (rawGate as Record<string, unknown>).simStability as number | undefined
          ?? (rawGate as Record<string, unknown>).sim_stability as number | undefined,
        simLiquidity: (rawGate as Record<string, unknown>).simLiquidity as number | undefined
          ?? (rawGate as Record<string, unknown>).sim_liquidity as number | undefined,
        simImpact: (rawGate as Record<string, unknown>).simImpact as number | undefined
          ?? (rawGate as Record<string, unknown>).sim_impact as number | undefined,
      }
      : undefined;

    // 提取 pre_sim_action / pre_sim_position_pct（顶层字段）
    const preSimAction = (result.pre_sim_action ?? result.preSimAction) as string | undefined;
    const preSimPositionPct = (result.pre_sim_position_pct ?? result.preSimPositionPct) as number | undefined;

    return {
      decision: result.decision,
      positionPct: (result.positionPct ?? result.position_pct ?? 0) as number,
      confidence: (result.confidence ?? 0) as number,
      riskLevel: (result.riskLevel ?? result.risk_level ?? "中") as string,
      stopLossPct: (result.stopLossPct ?? result.stop_loss_pct ?? 0) as number,
      takeProfitPct: (result.takeProfitPct ?? result.take_profit_pct ?? 0) as number,
      reasoning: (result.reasoning ?? "") as string,
      decisionTrail,
      technicalVeto,
      simulationGate,
      preSimAction,
      preSimPositionPct,
    };
  }

  return null;
}

// ── 主组件 ──

export function WhatIfBacktest() {
  const { t } = useTranslation();
  const [records, setRecords] = useState<AnalysisRecord[]>([]);
  const [loading, setLoading] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [snapshot, setSnapshot] = useState<Record<string, unknown> | null>(null);
  const [originalDecision, setOriginalDecision] = useState<PmDecision | null>(null);
  const [params, setParams] = useState<PmInputParams>(DEFAULT_PARAMS);
  const [result, setResult] = useState<PmDecision | null>(null);
  const [configOverrides, setConfigOverrides] = useState<Record<string, number>>({});
  const [toolReplayLoading, setToolReplayLoading] = useState(false);
  const [replayResult, setReplayResult] = useState<ToolChainReplayResult | null>(null);

  /** 把参数名解析为「i18n 标签 + 默认值 + 控件范围」。
   *
   * 默认值与标签均取自设置面板的单一权威源 `getDefaultVariables()`，
   * 此处不复制第二份清单 —— 否则同一参数会出现两套默认值（后端与面板分叉的
   * `risk_max_drawdown_limit` 20/15 就是这么来的）。 */
  const whatIfParamGroups = useMemo(() => {
    const byName = new Map(getDefaultVariables().map((v) => [v.name, v]));
    return WHATIF_PARAM_GROUPS.map((group) => ({
      titleKey: group.titleKey,
      items: group.params.map((spec) => {
        const meta = byName.get(spec.name);
        return {
          ...spec,
          labelKey: meta?.description,
          defaultValue: typeof meta?.value === "number" ? (meta.value as number) : undefined,
        };
      }),
    }));
  }, []);

  // 加载历史分析列表
  useEffect(() => {
    let cancelled = false;
    Promise.resolve().then(() => {
      if (cancelled) { return; }
      setLoading(true);
      return invoke<AnalysisRecord[]>("list_stock_analyses", { limit: 50, offset: 0 });
    })
      .then((list) => {
        if (!cancelled) {
          setRecords(list ?? []);
          // 自动选择第一条
          if (list && list.length > 0 && !selectedId) {
            setSelectedId(list[0].id);
          }
        }
      })
      .catch((e) => {
        if (!cancelled) { console.error("Failed to load analyses:", e); }
      })
      .finally(() => {
        if (!cancelled) { setLoading(false); }
      });
    return () => {
      cancelled = true;
    };
  }, [selectedId]);

  // 选择分析 → 加载 blackboard snapshot
  useEffect(() => {
    if (!selectedId) { return; }
    let cancelled = false;
    invoke<AnalysisRecord>("get_stock_analysis", { analysisId: selectedId })
      .then((record) => {
        if (cancelled || !record) { return; }
        // 解析 blackboard_snapshot
        let snap: Record<string, unknown> = {};
        try {
          snap = JSON.parse(record.blackboardSnapshot ?? "{}");
        } catch { /* empty */ }
        setSnapshot(snap);

        // 提取原始 params
        const extracted = extractParamsFromSnapshot(snap);
        setParams(extracted);

        // 提取原始决策
        const orig = originalDecisionSummary(snap);
        setOriginalDecision(orig);
      })
      .catch((e) => {
        if (!cancelled) { console.error("Failed to load analysis:", e); }
      });
    return () => {
      cancelled = true;
    };
  }, [selectedId]);

  // 每次 params 变化时重新计算（优先后端携带快照，fallback TS）
  useEffect(() => {
    let cancelled = false;
    (async () => {
      const backendResult = await computeDecisionBackend(params, snapshot);
      if (cancelled) { return; }
      if (backendResult) {
        setResult(backendResult);
      } else {
        setResult(computeDecisionLocal(params));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [params, snapshot]);

  // 选择框的 options
  const selectOptions: SelectProps["options"] = useMemo(() => {
    return records.map((r) => ({
      value: r.id,
      label: `[${r.stockCode}] ${r.stockName} — ${new Date(r.createdAt).toLocaleDateString("zh-CN")}`,
    }));
  }, [records]);

  const handleReset = useCallback(() => {
    if (snapshot) {
      setParams(extractParamsFromSnapshot(snapshot));
    }
  }, [snapshot]);

  // 判断 params 是否有变化
  const hasChanges = useMemo(() => {
    if (!snapshot) { return false; }
    const original = extractParamsFromSnapshot(snapshot);
    return JSON.stringify(original) !== JSON.stringify(params);
  }, [params, snapshot]);

  // 差异比较
  const diffFields = useMemo(() => {
    if (!originalDecision || !result) { return []; }
    const fields: { label: string; before: string; after: string; changed: boolean }[] = [];
    const add = (label: string, before: unknown, after: unknown) => {
      const bs = String(before ?? "—");
      const as = String(after ?? "—");
      fields.push({ label, before: bs, after: as, changed: bs !== as });
    };
    add(t("stockAnalysis.decision.action"), originalDecision.decision, result.decision);
    add(t("stockAnalysis.decision.confidence"), `${originalDecision.confidence}%`, `${result.confidence}%`);
    add(t("stockAnalysis.decision.positionPct"), `${originalDecision.positionPct}%`, `${result.positionPct}%`);
    add(t("stockAnalysis.decision.riskLevel"), originalDecision.riskLevel, result.riskLevel);
    return fields;
  }, [originalDecision, result, t]);

  return (
    <Card
      size="small"
      title={<span>🔬 {t("stockAnalysis.whatIfBacktest.title")}</span>}
      styles={{ body: { padding: "10px 12px" } }}
    >
      {/* Step 1: 选择历史分析 */}
      <div className="mb-3">
        <div className="text-xs text-gray-500 mb-1">{t("stockAnalysis.whatIfBacktest.step1")}</div>
        <Select
          className="w-full"
          size="small"
          placeholder={loading ? t("stockAnalysis.loading") : t("stockAnalysis.whatIfBacktest.selectHint")}
          loading={loading}
          value={selectedId}
          onChange={setSelectedId}
          options={selectOptions}
          showSearch
          filterOption={(input, option) =>
            (option?.label as string)?.toLowerCase().includes(input.toLowerCase()) ?? false}
        />
      </div>

      {!selectedId && (
        <Empty description={t("stockAnalysis.whatIfBacktest.selectEmpty")} image={Empty.PRESENTED_IMAGE_SIMPLE} />
      )}

      {snapshot && (
        <>
          {/* Step 2: 参数编辑 */}
          <div className="mb-3">
            <div className="text-xs text-gray-500 mb-1">{t("stockAnalysis.whatIfBacktest.step2")}</div>
            <div className="bg-gray-800/30 rounded p-2 space-y-2">
              <ParamSlider
                label={t("stockAnalysis.whatIfBacktest.totalScoreLabel")}
                value={params.totalScore}
                min={0}
                max={100}
                onChange={(v) => setParams((p) => ({ ...p, totalScore: v }))}
              />
              <ParamSlider
                label={t("stockAnalysis.whatIfBacktest.dqiScoreLabel")}
                value={params.dqiScore}
                min={0}
                max={100}
                onChange={(v) => setParams((p) => ({ ...p, dqiScore: v }))}
              />
              <ParamSelect
                label={t("stockAnalysis.whatIfBacktest.overallRiskLabel")}
                value={params.overallRisk}
                options={[
                  { value: "低", label: t("stockAnalysis.whatIfBacktest.riskLevels.low") },
                  { value: "中", label: t("stockAnalysis.whatIfBacktest.riskLevels.medium") },
                  { value: "高", label: t("stockAnalysis.whatIfBacktest.riskLevels.high") },
                  { value: "极高", label: t("stockAnalysis.whatIfBacktest.riskLevels.veryHigh") },
                ]}
                onChange={(v) => setParams((p) => ({ ...p, overallRisk: v }))}
              />
              <ParamSelect
                label={t("stockAnalysis.whatIfBacktest.catalystLevelLabel")}
                value={params.catalystLevel}
                options={[
                  { value: "无催化剂", label: t("stockAnalysis.whatIfBacktest.catalystLevels.none") },
                  { value: "L1普通消息", label: t("stockAnalysis.whatIfBacktest.catalystLevels.l1") },
                  { value: "L2业绩拐点级", label: t("stockAnalysis.whatIfBacktest.catalystLevels.l2") },
                  { value: "L3估值体系级", label: t("stockAnalysis.whatIfBacktest.catalystLevels.l3") },
                ]}
                onChange={(v) => setParams((p) => ({ ...p, catalystLevel: v }))}
              />
              <ParamSlider
                label={t("stockAnalysis.whatIfBacktest.consensusScoreLabel")}
                value={params.consensusScore}
                min={0}
                max={100}
                onChange={(v) => setParams((p) => ({ ...p, consensusScore: v }))}
              />
              <div className="flex justify-end gap-1">
                <Button size="small" onClick={handleReset} disabled={!hasChanges}>
                  {t("stockAnalysis.experiment.reset")}
                </Button>
              </div>
            </div>
          </div>

          {/* Config Overrides — 工具链配置参数覆盖回测 */}
          <Collapse
            ghost
            size="small"
            items={[{
              key: "configOverrides",
              label: (
                <span className="text-xs font-medium">{t("stockAnalysis.whatIfBacktest.configOverridesTitle")}</span>
              ),
              extra: toolReplayLoading
                ? <span className="text-xs text-blue-400">{t("stockAnalysis.whatIfBacktest.calculating")}</span>
                : undefined,
              children: (
                <div className="space-y-2">
                  <div className="text-[10px] text-gray-500">
                    {t("stockAnalysis.whatIfBacktest.configOverridesDesc")}
                  </div>
                  {
                    /* 参数按「后端真实消费方」分组渲染。滑块名与后端消费 key 逐字一致，
                      不再有「面板一套命名、引擎另一套命名」的空接线。 */
                  }
                  {whatIfParamGroups.map((group) => (
                    <div key={group.titleKey}>
                      <div className="text-[10px] text-gray-500 mt-1">{t(group.titleKey)}</div>
                      <div className="grid grid-cols-2 gap-2">
                        {group.items.map((item) => (
                          <ConfigParamSlider
                            key={item.name}
                            label={item.labelKey ? t(item.labelKey) : item.name}
                            value={configOverrides[item.name] ?? item.defaultValue}
                            onChange={(v) => setConfigOverrides((p) => ({ ...p, [item.name]: v }))}
                            min={item.min}
                            max={item.max}
                            step={item.step}
                          />
                        ))}
                      </div>
                    </div>
                  ))}
                  <div className="flex justify-end">
                    <Button
                      size="small"
                      type="primary"
                      loading={toolReplayLoading}
                      onClick={async () => {
                        setToolReplayLoading(true);
                        try {
                          const _stockCode = selectedId
                            ? records.find((r) => r.id === selectedId)?.stockCode
                            : "";
                          if (!_stockCode) {
                            setReplayResult({
                              totalScore: 0,
                              decision: "—",
                              positionPct: 0,
                              riskLevel: "—",
                              dataDegraded: false,
                              error: t("stockAnalysis.whatIfBacktest.replayNoSelection"),
                            });
                            // 提前 return 前必须复位 loading，否则按钮永久转圈
                            setToolReplayLoading(false);
                            return;
                          }
                          const raw = await invoke("replay_tool_chain", {
                            params: { stockCode: _stockCode, configOverrides },
                          }) as Record<string, unknown> | null;
                          const decision = (raw?.decision ?? {}) as Record<string, unknown>;
                          setReplayResult({
                            totalScore: typeof raw?.totalScore === "number" ? raw.totalScore : 0,
                            decision: String(decision.decision ?? "—"),
                            positionPct: typeof decision.positionPct === "number"
                              ? Math.round(decision.positionPct)
                              : 0,
                            riskLevel: String(decision.riskLevel ?? "—"),
                            dataDegraded: raw?.dataDegraded === true,
                          });
                        } catch (e) {
                          console.error("Tool chain replay failed:", e);
                          setReplayResult({
                            totalScore: 0,
                            decision: "—",
                            positionPct: 0,
                            riskLevel: "—",
                            dataDegraded: false,
                            error: t("stockAnalysis.whatIfBacktest.replayFailed"),
                          });
                        }
                        setToolReplayLoading(false);
                      }}
                    >
                      {t("stockAnalysis.whatIfBacktest.applyToBackend")}
                    </Button>
                  </div>
                  {replayResult && (
                    <div className="text-[10px] space-y-0.5 border-t border-gray-700 pt-1 mt-1">
                      <div className="text-gray-400">
                        {t("stockAnalysis.whatIfBacktest.replayResultTitle")}
                      </div>
                      {replayResult.error
                        ? <div className="text-amber-400">{replayResult.error}</div>
                        : (
                          <>
                            <div className="flex justify-between">
                              <span className="text-gray-500">
                                {t("stockAnalysis.whatIfBacktest.replayTotalScore")}
                              </span>
                              <span className="text-gray-200">{replayResult.totalScore}</span>
                            </div>
                            <div className="flex justify-between">
                              <span className="text-gray-500">{t("stockAnalysis.decision.action")}</span>
                              <span className="text-gray-200">{replayResult.decision}</span>
                            </div>
                            <div className="flex justify-between">
                              <span className="text-gray-500">{t("stockAnalysis.decision.positionPct")}</span>
                              <span className="text-gray-200">{replayResult.positionPct}%</span>
                            </div>
                            <div className="flex justify-between">
                              <span className="text-gray-500">{t("stockAnalysis.decision.riskLevel")}</span>
                              <span className="text-gray-200">{replayResult.riskLevel}</span>
                            </div>
                            {replayResult.dataDegraded && (
                              <div className="text-amber-400">
                                {t("stockAnalysis.whatIfBacktest.replayDataDegraded")}
                              </div>
                            )}
                          </>
                        )}
                    </div>
                  )}
                </div>
              ),
            }]}
          />

          {/* Step 3: 对比结果 */}
          <div>
            <div className="text-xs text-gray-500 mb-1">
              {t("stockAnalysis.whatIfBacktest.step3")}
              {hasChanges && (
                <Tag color="blue" className="ml-1 text-[10px]!">{t("stockAnalysis.whatIfBacktest.modified")}</Tag>
              )}
            </div>

            {originalDecision && result && (
              <div className="space-y-1">
                {diffFields.map((f) => (
                  <div
                    key={f.label}
                    className={`flex items-center justify-between px-2 py-1 rounded text-xs ${
                      f.changed ? "bg-blue-900/20" : ""
                    }`}
                  >
                    <span className="text-gray-400 w-24">{f.label}</span>
                    <div className="flex items-center gap-1 flex-1 justify-end">
                      <span className={f.changed ? "text-gray-500 line-through" : "text-gray-300"}>
                        {f.before}
                      </span>
                      {f.changed && (
                        <>
                          <ArrowRightOutlined className="text-blue-400 text-[10px]!" />
                          <span className="text-blue-300 font-medium">{f.after}</span>
                        </>
                      )}
                    </div>
                  </div>
                ))}

                {result.reasoning && (
                  <div className="mt-2 px-2 py-1.5 bg-gray-800/40 rounded text-[11px] text-gray-400 leading-relaxed">
                    {result.reasoning}
                  </div>
                )}

                {/* 决策追溯链（DAG 完整输出时展示） */}
                {result.decisionTrail && result.decisionTrail.length > 0 && (
                  <div className="mt-2">
                    <div className="text-[10px] text-gray-500 mb-1 uppercase tracking-wider">
                      {t("stockAnalysis.whatIf.decisionTrail")}
                    </div>
                    <div className="space-y-0.5">
                      {result.decisionTrail.map((t, i) => (
                        <div key={i} className="flex items-center gap-1.5 text-[11px]">
                          <span
                            className={`inline-block w-1.5 h-1.5 rounded-full ${
                              t.status === "VETOED"
                                ? "bg-red-500"
                                : t.status === "DOWNGRADED"
                                ? "bg-amber-500"
                                : t.status === "PASS"
                                ? "bg-green-500"
                                : "bg-gray-500"
                            }`}
                          />
                          <span className="text-gray-400 font-mono">{t.ruleId}</span>
                          <span className="text-gray-500">{t.status}</span>
                          {t.detail && <span className="text-gray-500">— {t.detail}</span>}
                        </div>
                      ))}
                    </div>
                  </div>
                )}

                {/* 技术面否决详情 */}
                {result.technicalVeto?.vetoed && (
                  <div className="mt-2 px-2 py-1.5 bg-red-900/20 rounded text-[11px] text-red-400">
                    ⛔ {t("stockAnalysis.whatIf.technicalVeto")}: {result.technicalVeto.ruleId ?? ""}
                    {result.technicalVeto.reason ? ` — ${result.technicalVeto.reason}` : ""}
                  </div>
                )}

                {/* 模拟门信息（S-501~503） */}
                {result.simulationGate?.vetoed && (
                  <div className="mt-2">
                    <div className="px-2 py-1.5 bg-amber-900/20 rounded text-[11px] text-amber-400">
                      🏭 {t("stockAnalysis.whatIf.simulationGate")} {result.simulationGate.ruleId ?? ""}:
                      {result.simulationGate.reason ? ` ${result.simulationGate.reason}` : ""}
                    </div>
                    {result.preSimAction && (
                      <div className="mt-1 px-2 py-1 text-[10px] text-gray-500">
                        {t("stockAnalysis.whatIf.preSimulationDecision")}: {result.preSimAction}
                        {result.preSimPositionPct != null ? ` @ ${result.preSimPositionPct}%` : ""}
                        {result.simulationGate.simStability != null
                          ? ` | ${t("stockAnalysis.whatIf.stability")} ${result.simulationGate.simStability}`
                          : ""}
                        {result.simulationGate.simLiquidity != null
                          ? ` | ${t("stockAnalysis.whatIf.liquidity")} ${result.simulationGate.simLiquidity}`
                          : ""}
                        {result.simulationGate.simImpact != null
                          ? ` | ${t("stockAnalysis.whatIf.impact")} ${result.simulationGate.simImpact}bps`
                          : ""}
                      </div>
                    )}
                  </div>
                )}
              </div>
            )}

            {!originalDecision && (
              <Empty
                description={t("stockAnalysis.whatIfBacktest.noOriginalDecision")}
                image={Empty.PRESENTED_IMAGE_SIMPLE}
              />
            )}
          </div>
        </>
      )}
    </Card>
  );
}

// ── 子组件 ──

/** 数值滑块参数控件 */
function ParamSlider({
  label,
  value,
  min,
  max,
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  onChange: (v: number) => void;
}) {
  return (
    <div className="flex items-center gap-2">
      <span className="text-xs text-gray-400 w-32 shrink-0">{label}</span>
      <Slider
        className="flex-1 mb-0!"
        min={min}
        max={max}
        value={value}
        onChange={onChange}
      />
      <InputNumber
        className="w-16!"
        size="small"
        min={min}
        max={max}
        value={value}
        onChange={(v) => onChange(v ?? min)}
        controls={false}
      />
    </div>
  );
}

/** 枚举选择参数控件 */
function ParamSelect({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: string;
  options: { value: string; label: string }[];
  onChange: (v: string) => void;
}) {
  return (
    <div className="flex items-center gap-2">
      <span className="text-xs text-gray-400 w-32 shrink-0">{label}</span>
      <Select
        className="flex-1"
        size="small"
        value={value}
        onChange={onChange}
        options={options}
      />
    </div>
  );
}

/** 配置参数覆盖滑块（L2 工具链回测） */
/** `replay_tool_chain` 回放结果的展示视图（仅取 UI 所需字段）。
 *
 * 修复前该命令的返回值被 `await` 后直接丢弃 —— 用户调完滑块点「应用配置到后端重算」
 * 看不到任何反馈，无法判断调参是否生效，等于「接线了但不可观测」。 */
interface ToolChainReplayResult {
  totalScore: number;
  decision: string;
  positionPct: number;
  riskLevel: string;
  dataDegraded: boolean;
  error?: string;
}

/** What-If 可调参数的控件范围。
 *
 * `name` **必须与后端消费 key 逐字一致**，否则滑块调了也不生效 —— 这正是本面板
 * 修复前的状态：面板写的是 `scoring_*`（有效）+ `value_dcf_*`（无效，估值输入来自
 * 快照而非重算），而决策层的 `action_*` / `pos_cap_*` / `regime_prior_*` 等真参数
 * 根本没有 UI 入口。 */
interface WhatIfParamSpec {
  name: string;
  min: number;
  max: number;
  step: number;
}

/** What-If 参数分组。每组标题点名该组参数的**真实消费链路**，
 * 便于日后排查「面板可调但引擎不读」的空接线。
 *
 * - 第 1 组由 `replay_tool_chain` 的 Rust 简化链直接消费；
 * - 第 2–6 组经 `WhatIfRequest.paramOverrides` 透传进 `portfolio-mgr.rhai`
 *   （权威源 = 后端 `PORTFOLIO_MGR_TUNABLE_PARAMS`，共 28 项）。 */
const WHATIF_PARAM_GROUPS: { titleKey: string; params: WhatIfParamSpec[] }[] = [
  {
    titleKey: "stockAnalysis.whatIfBacktest.groupScoringWeights",
    params: [
      { name: "scoring_trend", min: 0, max: 60, step: 1 },
      { name: "scoring_deviation", min: 0, max: 60, step: 1 },
      { name: "scoring_macd", min: 0, max: 60, step: 1 },
      { name: "scoring_volume", min: 0, max: 60, step: 1 },
      { name: "scoring_rsi", min: 0, max: 60, step: 1 },
      { name: "scoring_support", min: 0, max: 60, step: 1 },
    ],
  },
  {
    titleKey: "stockAnalysis.whatIfBacktest.groupActionThresholds",
    params: [
      { name: "action_buy_threshold", min: 0, max: 1, step: 0.01 },
      { name: "action_increase_threshold", min: 0, max: 1, step: 0.01 },
      { name: "action_hold_threshold", min: 0, max: 1, step: 0.01 },
      { name: "action_watch_threshold", min: 0, max: 1, step: 0.01 },
      { name: "action_reduce_threshold", min: 0, max: 1, step: 0.01 },
    ],
  },
  {
    titleKey: "stockAnalysis.whatIfBacktest.groupPositionCaps",
    params: [
      { name: "pos_buy_min", min: 0, max: 100, step: 1 },
      { name: "pos_increase_min", min: 0, max: 100, step: 1 },
      { name: "pos_cap_extreme", min: 0, max: 100, step: 1 },
      { name: "pos_cap_high", min: 0, max: 100, step: 1 },
      { name: "pos_cap_mid", min: 0, max: 100, step: 1 },
    ],
  },
  {
    titleKey: "stockAnalysis.whatIfBacktest.groupRegimePriors",
    params: [
      { name: "regime_prior_bull", min: 0, max: 1, step: 0.01 },
      { name: "regime_prior_sideways", min: 0, max: 1, step: 0.01 },
      { name: "regime_prior_bear", min: 0, max: 1, step: 0.01 },
    ],
  },
  {
    titleKey: "stockAnalysis.whatIfBacktest.groupRiskThresholds",
    params: [
      { name: "risk_debt_extreme", min: 0, max: 100, step: 1 },
      { name: "risk_vol_extreme", min: 0, max: 100, step: 1 },
      { name: "risk_sharpe_extreme", min: -5, max: 5, step: 0.1 },
      { name: "risk_vol_high", min: 0, max: 100, step: 1 },
      { name: "risk_dd_high", min: 0, max: 100, step: 1 },
      { name: "risk_roe_high", min: -20, max: 40, step: 1 },
      { name: "risk_debt_high", min: 0, max: 100, step: 1 },
      { name: "risk_vol_low", min: 0, max: 100, step: 1 },
      { name: "risk_sharpe_low", min: -5, max: 5, step: 0.1 },
      { name: "risk_dd_low", min: 0, max: 100, step: 1 },
      { name: "risk_roe_low", min: -20, max: 40, step: 1 },
      { name: "risk_debt_low", min: 0, max: 100, step: 1 },
      { name: "risk_growth_low", min: -50, max: 100, step: 1 },
    ],
  },
  {
    titleKey: "stockAnalysis.whatIfBacktest.groupMiscParams",
    params: [
      { name: "kelly_fraction", min: 0, max: 1, step: 0.05 },
      { name: "risk_max_drawdown_limit", min: 0, max: 50, step: 1 },
      { name: "trader_cap_min_weight", min: 0, max: 0.5, step: 0.01 },
      { name: "cost_pct", min: 0, max: 0.05, step: 0.001 },
      { name: "value_fscore_buy", min: 0, max: 9, step: 1 },
    ],
  },
];

function ConfigParamSlider({
  label,
  value,
  onChange,
  min = 0,
  max = 100,
  step,
}: {
  label: string;
  value?: number;
  onChange: (v: number) => void;
  min?: number;
  max?: number;
  step?: number;
}) {
  return (
    <div className="flex items-center gap-1">
      <span className="text-[10px] text-gray-500 w-20 truncate" title={label}>{label}</span>
      <Slider
        className="flex-1 mb-0!"
        min={min}
        max={max}
        step={step ?? 1}
        value={value ?? 50}
        onChange={onChange}
      />
      <InputNumber
        className="w-14! text-[11px]!"
        size="small"
        min={min}
        max={max}
        step={step ?? 1}
        value={value ?? 50}
        onChange={(v) => onChange(v ?? min)}
        controls={false}
      />
    </div>
  );
}
