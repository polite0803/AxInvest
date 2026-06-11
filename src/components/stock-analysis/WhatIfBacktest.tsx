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
  blackboardSnapshot: string | null;
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
  institutionalTrace: string;
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
}

// ── 默认值 ──

const DEFAULT_PARAMS: PmInputParams = {
  totalScore: 50,
  dqiScore: 50,
  overallRisk: "中",
  catalystLevel: "无催化剂",
  institutionalTrace: "无异常",
  consensusScore: 50,
};

// ── 前端 Rhai 公式移植（与 portfolio-mgr.rhai 保持一致）──
// 优先调用后端 Rhai 引擎，fallback 到本地 TS 版本

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

async function computeDecisionBackend(params: PmInputParams): Promise<PmDecision | null> {
  try {
    const result = await invoke<any>("compute_what_if", {
      params: {
        totalScore: params.totalScore,
        dqiScore: params.dqiScore,
        overallRisk: params.overallRisk,
        catalystLevel: params.catalystLevel,
        institutionalTrace: params.institutionalTrace,
        consensusScore: params.consensusScore,
      },
    });
    if (result) {
      return {
        decision: result.decision,
        positionPct: Math.round(result.positionPct),
        confidence: Math.round(result.confidence),
        riskLevel: result.riskLevel,
        stopLossPct: result.stopLossPct,
        takeProfitPct: result.takeProfitPct,
        reasoning: result.reasoning,
      };
    }
  } catch (e) {
    console.warn("Backend formula failed, falling back to TS:", e);
  }
  return null;
}

function computeDecisionLocal(params: PmInputParams): PmDecision {
  const { totalScore, dqiScore, overallRisk, catalystLevel, institutionalTrace, consensusScore } = params;

  // 辩论收敛调整
  const consensusAdj = ((consensusScore - 50) / 100) * 10;

  // 数据质量调整
  const dqiAdj = ((dqiScore - 50) / 100) * 5;

  // 风险调整
  const riskAdjustment = (() => {
    switch (overallRisk) {
      case "低": return 5;
      case "高": return -5;
      case "极高": return -10;
      default: return 0;
    }
  })();

  // 催化剂加成
  const catalystBonus = (() => {
    switch (catalystLevel) {
      case "L3估值体系级": return 12;
      case "L2业绩拐点级": return 6;
      case "L1普通消息": return 2;
      default: return 0;
    }
  })();

  // 机构建仓痕迹
  const instBonus = (institutionalTrace === "有建仓痕迹" || institutionalTrace === "疑似建仓") ? 5 : 0;

  // 最终置信度
  const adjustment = consensusAdj + dqiAdj + riskAdjustment + catalystBonus + instBonus;
  const confidence = clamp(totalScore + adjustment, 0, 100);

  // 仓位推导
  const basePos = riskAdjustment >= 0 ? confidence * 0.8 : confidence * 0.5;
  const positionPct = clamp(basePos, 0, 100);

  // 最终动作
  const decision = (() => {
    if (confidence >= 80 && positionPct >= 30) return "增持";
    if (confidence >= 60) return "买入";
    if (confidence >= 40) return "持有";
    if (positionPct < 10) return "减持";
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
    reasoning: `确定性公式结果: totalScore=${totalScore.toFixed(0)}, dqi=${dqiScore.toFixed(0)}, risk=${overallRisk}, catalyst=${catalystLevel}, consensus=${consensusScore}, adjustment=${adjustment.toFixed(1)}, confidence=${Math.round(confidence)}, position=${Math.round(positionPct)}`,
  };
}

// ── 参数解析 ──

/** 从 blackboard_snapshot 中提取 portfolio-mgr 输入参数
 *
 * Phase 5: 优先从 `params.portfolio-mgr.input_params` 读取（CodeNode 直接保存的
 * 原始 input_mapping 解析值快照），fallback 到从各上游节点 params 重建。
 */
function extractParamsFromSnapshot(snapshot: Record<string, any>): PmInputParams {
  const params: PmInputParams = { ...DEFAULT_PARAMS };

  // Phase 5: 优先从 params.portfolio-mgr.input_params 读取
  // 这是 code_executor.rs 直接保存的 input_mapping 解析值快照
  const pmParams = snapshot["params.portfolio-mgr"];
  const inputParams = pmParams?.input_params;
  if (inputParams) {
    if (typeof inputParams.totalScore === "number") params.totalScore = inputParams.totalScore;
    if (typeof inputParams.dqiScore === "number") params.dqiScore = inputParams.dqiScore;
    if (typeof inputParams.overallRisk === "string") params.overallRisk = inputParams.overallRisk;
    if (typeof inputParams.catalystLevel === "string") params.catalystLevel = inputParams.catalystLevel;
    if (typeof inputParams.institutionalTrace === "string") params.institutionalTrace = inputParams.institutionalTrace;
    if (typeof inputParams.consensusScore === "number") params.consensusScore = inputParams.consensusScore;
    return params; // input_params 有完整快照，直接返回
  }

  // Fallback: 从 params.portfolio-mgr result 反推
  if (pmParams && typeof pmParams.totalScore === "number") {
    params.totalScore = pmParams.totalScore;
  }
  if (pmParams && typeof pmParams.dqiScore === "number") {
    params.dqiScore = pmParams.dqiScore;
  }

  // 从 params.data-quality 读取 dqi_score
  const dqParams = snapshot["params.data-quality"];
  if (dqParams && typeof dqParams.score === "number") {
    params.dqiScore = dqParams.score;
  }

  // 从 params.a-catalyst 读取催化剂参数
  const catParams = snapshot["params.a-catalyst"];
  if (catParams) {
    if (catParams.catalyst_level) params.catalystLevel = catParams.catalyst_level;
    if (catParams.institutional_trace) params.institutionalTrace = catParams.institutional_trace;
  }

  // 尝试从决策 JSON 反推 totalScore
  const decoded = tryParseDecisionJson(snapshot["portfolio-mgr"]);
  if (decoded && typeof decoded.confidence === "number") {
    // 保留用户已有的决策值作为参考，但 params 优先
  }

  return params;
}

function tryParseDecisionJson(input: any): any {
  if (!input) return null;
  // 可能是字符串 JSON，也可能是对象
  if (typeof input === "string") {
    try { return JSON.parse(input); } catch { return null; }
  }
  return input;
}

// ── UI 组件 ──

/** 原始决策摘要 */
function originalDecisionSummary(snapshot: Record<string, any>): PmDecision | null {
  // portfolio-mgr 的 output
  const pmOutput = snapshot["portfolio-mgr"];
  if (!pmOutput) return null;

  const parsed = tryParseDecisionJson(pmOutput);
  if (!parsed || !parsed.decision && !parsed.result) return null;

  // CodeNode 的 result 在 output.result 中
  const result = parsed.result || parsed;

  // 从 decision_json 解析
  if (typeof result.decision === "string") {
    return {
      decision: result.decision,
      positionPct: result.positionPct ?? result.position_pct ?? 0,
      confidence: result.confidence ?? 0,
      riskLevel: result.riskLevel ?? result.risk_level ?? "中",
      stopLossPct: result.stopLossPct ?? result.stop_loss_pct ?? 0,
      takeProfitPct: result.takeProfitPct ?? result.take_profit_pct ?? 0,
      reasoning: result.reasoning ?? "",
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
  const [snapshot, setSnapshot] = useState<Record<string, any> | null>(null);
  const [originalDecision, setOriginalDecision] = useState<PmDecision | null>(null);
  const [params, setParams] = useState<PmInputParams>(DEFAULT_PARAMS);
  const [result, setResult] = useState<PmDecision | null>(null);
  const [configOverrides, setConfigOverrides] = useState<Record<string, number>>({});
  const [toolReplayLoading, setToolReplayLoading] = useState(false);

  // 加载历史分析列表
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    invoke<AnalysisRecord[]>("list_stock_analyses", { limit: 50, offset: 0 })
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
        if (!cancelled) console.error("Failed to load analyses:", e);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => { cancelled = true; };
  }, []);

  // 选择分析 → 加载 blackboard snapshot
  useEffect(() => {
    if (!selectedId) return;
    let cancelled = false;
    invoke<AnalysisRecord>("get_stock_analysis", { analysisId: selectedId })
      .then((record) => {
        if (cancelled || !record) return;
        // 解析 blackboard_snapshot
        let snap: Record<string, any> = {};
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
        if (!cancelled) console.error("Failed to load analysis:", e);
      });
    return () => { cancelled = true; };
  }, [selectedId]);

  // 每次 params 变化时重新计算（优先后端，fallback TS）
  useEffect(() => {
    let cancelled = false;
    (async () => {
      const backendResult = await computeDecisionBackend(params);
      if (cancelled) return;
      if (backendResult) {
        setResult(backendResult);
      } else {
        setResult(computeDecisionLocal(params));
      }
    })();
    return () => { cancelled = true; };
  }, [params]);

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
    if (!snapshot) return false;
    const original = extractParamsFromSnapshot(snapshot);
    return JSON.stringify(original) !== JSON.stringify(params);
  }, [params, snapshot]);

  // 差异比较
  const diffFields = useMemo(() => {
    if (!originalDecision || !result) return [];
    const fields: { label: string; before: string; after: string; changed: boolean }[] = [];
    const add = (label: string, before: any, after: any) => {
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
      title={<span>🔬 What-If Param Backtest</span>}
      styles={{ body: { padding: "10px 12px" } }}
    >
      {/* Step 1: 选择历史分析 */}
      <div className="mb-3">
        <div className="text-xs text-gray-500 mb-1">Step 1: 选择历史分析</div>
        <Select
          className="w-full"
          size="small"
          placeholder={loading ? t("stockAnalysis.loading") : "选择一条分析记录..."}
          loading={loading}
          value={selectedId}
          onChange={setSelectedId}
          options={selectOptions}
          showSearch
          filterOption={(input, option) =>
            (option?.label as string)?.toLowerCase().includes(input.toLowerCase()) ?? false
          }
        />
      </div>

      {!selectedId && (
        <Empty description="请选择一条历史分析记录开始 What-If 回测" image={Empty.PRESENTED_IMAGE_SIMPLE} />
      )}

      {snapshot && (
        <>
          {/* Step 2: 参数编辑 */}
          <div className="mb-3">
            <div className="text-xs text-gray-500 mb-1">Step 2: 调整参数</div>
            <div className="bg-gray-800/30 rounded p-2 space-y-2">
              <ParamSlider
                label="totalScore（基础评分）"
                value={params.totalScore}
                min={0} max={100}
                onChange={(v) => setParams((p) => ({ ...p, totalScore: v }))}
              />
              <ParamSlider
                label="dqiScore（数据质量）"
                value={params.dqiScore}
                min={0} max={100}
                onChange={(v) => setParams((p) => ({ ...p, dqiScore: v }))}
              />
              <ParamSelect
                label="overallRisk（风险等级）"
                value={params.overallRisk}
                options={[
                  { value: "低", label: "低" },
                  { value: "中", label: "中" },
                  { value: "高", label: "高" },
                  { value: "极高", label: "极高" },
                ]}
                onChange={(v) => setParams((p) => ({ ...p, overallRisk: v }))}
              />
              <ParamSelect
                label="catalystLevel（催化剂级别）"
                value={params.catalystLevel}
                options={[
                  { value: "无催化剂", label: "无催化剂" },
                  { value: "L1普通消息", label: "L1 普通消息" },
                  { value: "L2业绩拐点级", label: "L2 业绩拐点级" },
                  { value: "L3估值体系级", label: "L3 估值体系级" },
                ]}
                onChange={(v) => setParams((p) => ({ ...p, catalystLevel: v }))}
              />
              <ParamSelect
                label="institutionalTrace（机构痕迹）"
                value={params.institutionalTrace}
                options={[
                  { value: "无异常", label: "无异常" },
                  { value: "疑似建仓", label: "疑似建仓" },
                  { value: "有建仓痕迹", label: "有建仓痕迹" },
                  { value: "资金出逃", label: "资金出逃" },
                ]}
                onChange={(v) => setParams((p) => ({ ...p, institutionalTrace: v }))}
              />
              <ParamSlider
                label="consensusScore（辩论收敛）"
                value={params.consensusScore}
                min={0} max={100}
                onChange={(v) => setParams((p) => ({ ...p, consensusScore: v }))}
              />
              <div className="flex justify-end gap-1">
                <Button size="small" onClick={handleReset} disabled={!hasChanges}>
                  重置
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
              label: <span className="text-xs font-medium">配置参数覆盖（评分权重/估值/风控）</span>,
              extra: toolReplayLoading ? <span className="text-xs text-blue-400">计算中...</span> : undefined,
              children: (
                <div className="space-y-2">
                  <div className="text-[10px] text-gray-500">修改这些参数会通过后端重新计算工具链（scoring + valuation + risk），进而影响 totalScore 和最终决策。</div>
                  <div className="grid grid-cols-2 gap-2">
                    <ConfigParamSlider label="scoring_trend" value={configOverrides.scoring_trend} onChange={(v) => setConfigOverrides((p) => ({ ...p, scoring_trend: v }))} />
                    <ConfigParamSlider label="scoring_deviation" value={configOverrides.scoring_deviation} onChange={(v) => setConfigOverrides((p) => ({ ...p, scoring_deviation: v }))} />
                    <ConfigParamSlider label="scoring_macd" value={configOverrides.scoring_macd} onChange={(v) => setConfigOverrides((p) => ({ ...p, scoring_macd: v }))} />
                    <ConfigParamSlider label="scoring_volume" value={configOverrides.scoring_volume} onChange={(v) => setConfigOverrides((p) => ({ ...p, scoring_volume: v }))} />
                    <ConfigParamSlider label="scoring_rsi" value={configOverrides.scoring_rsi} onChange={(v) => setConfigOverrides((p) => ({ ...p, scoring_rsi: v }))} />
                    <ConfigParamSlider label="scoring_support" value={configOverrides.scoring_support} onChange={(v) => setConfigOverrides((p) => ({ ...p, scoring_support: v }))} />
                  </div>
                  <div className="text-[10px] text-gray-500 mt-1">估值参数</div>
                  <div className="grid grid-cols-2 gap-2">
                    <ConfigParamSlider label="value_dcf_growth_rate" value={configOverrides.value_dcf_growth_rate} onChange={(v) => setConfigOverrides((p) => ({ ...p, value_dcf_growth_rate: v }))} />
                    <ConfigParamSlider label="value_dcf_discount_rate" value={configOverrides.value_dcf_discount_rate} onChange={(v) => setConfigOverrides((p) => ({ ...p, value_dcf_discount_rate: v }))} />
                    <ConfigParamSlider label="value_safety_margin" value={configOverrides.value_safety_margin} onChange={(v) => setConfigOverrides((p) => ({ ...p, value_safety_margin: v }))} />
                  </div>
                  <div className="text-[10px] text-gray-500 mt-1">风控参数</div>
                  <div className="grid grid-cols-2 gap-2">
                    <ConfigParamSlider label="kelly_fraction" value={configOverrides.kelly_fraction} onChange={(v) => setConfigOverrides((p) => ({ ...p, kelly_fraction: v }))} min={0} max={1} step={0.05} />
                    <ConfigParamSlider label="risk_max_drawdown_limit" value={configOverrides.risk_max_drawdown_limit} onChange={(v) => setConfigOverrides((p) => ({ ...p, risk_max_drawdown_limit: v }))} min={5} max={50} />
                  </div>
                  <div className="flex justify-end">
                    <Button
                      size="small"
                      type="primary"
                      loading={toolReplayLoading}
                      onClick={async () => {
                        setToolReplayLoading(true);
                        try {
                          const _stockCode = selectedId ? records.find((r) => r.id === selectedId)?.stockCode : "";
                          if (!_stockCode) return;
                          await invoke("replay_tool_chain", {
                            params: { stockCode: _stockCode, configOverrides },
                          });
                        } catch (e) {
                          console.error("Tool chain replay failed:", e);
                        }
                        setToolReplayLoading(false);
                      }}
                    >
                      应用配置到后端重算
                    </Button>
                  </div>
                </div>
              ),
            }]}
          />

          {/* Step 3: 对比结果 */}
          <div>
            <div className="text-xs text-gray-500 mb-1">
              Step 3: 对比结果
              {hasChanges && <Tag color="blue" className="ml-1 !text-[10px]">已修改</Tag>}
            </div>

            {originalDecision && result && (
              <div className="space-y-1">
                {diffFields.map((f) => (
                  <div key={f.label} className={`flex items-center justify-between px-2 py-1 rounded text-xs ${
                    f.changed ? "bg-blue-900/20" : ""
                  }`}>
                    <span className="text-gray-400 w-24">{f.label}</span>
                    <div className="flex items-center gap-1 flex-1 justify-end">
                      <span className={f.changed ? "text-gray-500 line-through" : "text-gray-300"}>
                        {f.before}
                      </span>
                      {f.changed && (
                        <>
                          <ArrowRightOutlined className="text-blue-400 !text-[10px]" />
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
              </div>
            )}

            {!originalDecision && (
              <Empty
                description="未找到原始决策数据（该分析可能未完成或无 portfolio-mgr 输出）"
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
  label, value, min, max, onChange,
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
        className="flex-1 !mb-0"
        min={min}
        max={max}
        value={value}
        onChange={onChange}
      />
      <InputNumber
        className="!w-16"
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
  label, value, options, onChange,
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
function ConfigParamSlider({
  label, value, onChange, min = 0, max = 100, step,
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
        className="flex-1 !mb-0"
        min={min} max={max}
        step={step ?? 1}
        value={value ?? 50}
        onChange={onChange}
      />
      <InputNumber
        className="!w-14 !text-[11px]"
        size="small"
        min={min} max={max}
        step={step ?? 1}
        value={value ?? 50}
        onChange={(v) => onChange(v ?? min)}
        controls={false}
      />
    </div>
  );
}
