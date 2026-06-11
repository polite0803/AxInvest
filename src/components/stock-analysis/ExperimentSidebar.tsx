/**
 * ExperimentSidebar — 实验模式侧栏
 *
 * 在 StockAnalysisPage 完成分析后，用户可切换至 Experiment 模式。
 * 侧栏显示 What-If 参数滑块、Config Overrides 折叠区、实时对比预览、
 * Accept/Reset 按钮。
 */

import { invoke } from "@/lib/invoke";
import { type ExperimentRecord, useStockAnalysisStore } from "@/stores/feature/stockAnalysisStore";
import { parseRiskLevel } from "@/types/stock-analysis";
import { Button, InputNumber, Select, Slider } from "antd";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

// ── 类型定义 ──

interface WhatIfParams {
  consensusScore: number;
  totalScore: number;
  dqiScore: number;
  overallRisk: string;
  catalystLevel: string;
  institutionalTrace: string;
}

const DEFAULT_PARAMS: WhatIfParams = {
  consensusScore: 50,
  totalScore: 50,
  dqiScore: 50,
  overallRisk: "中",
  catalystLevel: "无催化剂",
  institutionalTrace: "无异常",
};

// ── 前端公式（与后端 Rhai 一致，fallback） ──

function computeDecision(params: WhatIfParams): {
  decision: string;
  confidence: number;
  positionPct: number;
  riskLevel: string;
} {
  const consensusAdj = ((params.consensusScore - 50) / 100) * 10;
  const dqiAdj = ((params.dqiScore - 50) / 100) * 5;
  const riskAdj = params.overallRisk === "低"
    ? 5
    : params.overallRisk === "高"
    ? -5
    : params.overallRisk === "极高"
    ? -10
    : 0;
  const catBonus = params.catalystLevel === "L3估值体系级"
    ? 12
    : params.catalystLevel === "L2业绩拐点级"
    ? 6
    : params.catalystLevel === "L1普通消息"
    ? 2
    : 0;
  const instBonus = params.institutionalTrace === "有建仓痕迹" || params.institutionalTrace === "疑似建仓" ? 5 : 0;
  const adj = consensusAdj + dqiAdj + riskAdj + catBonus + instBonus;
  const confidence = Math.max(0, Math.min(100, params.totalScore + adj));
  const basePos = riskAdj >= 0 ? confidence * 0.8 : confidence * 0.5;
  const pos = Math.max(0, Math.min(100, basePos));
  const decision = confidence >= 80 && pos >= 30
    ? "buy"
    : confidence >= 60
    ? "buy"
    : confidence >= 40
    ? "hold"
    : pos < 10
    ? "sell"
    : "hold";
  return { decision, confidence: Math.round(confidence), positionPct: Math.round(pos), riskLevel: params.overallRisk };
}

// ── 组件 ──

export function ExperimentSidebar() {
  const decision = useStockAnalysisStore((s) => s.decision);
  const pushExperiment = useStockAnalysisStore((s) => s.pushExperiment);
  const experiments = useStockAnalysisStore((s) => s.experiments);
  const stockCode = useStockAnalysisStore((s) => s.stockCode);

  // Initialize params from original decision
  const originalDecision = useMemo(() =>
    decision
      ? {
        decision: decision.action,
        confidence: decision.confidence,
        positionPct: decision.positionPct,
        riskLevel: decision.riskLevel,
      }
      : null, [decision]);

  const [params, setParams] = useState<WhatIfParams>(DEFAULT_PARAMS);
  const [configOverrides, setConfigOverrides] = useState<Record<string, number>>({});
  const [toolReplayLoading, setToolReplayLoading] = useState(false);
  const [showConfig, setShowConfig] = useState(false);
  const paramRef = useRef(params);

  // Sync original decision into params on first load
  useEffect(() => {
    if (decision && paramRef.current === DEFAULT_PARAMS) {
      setParams((p) => ({
        ...p,
        totalScore: typeof decision.confidence === "number" ? decision.confidence : 50,
        overallRisk: decision.riskLevel || "中",
      }));
    }
  }, [decision]);

  // Keep ref in sync
  useEffect(() => {
    paramRef.current = params;
  }, [params]);

  // Compute live result
  const result = useMemo(() => computeDecision(params), [params]);

  // Track if params differ from original
  const hasChanges = useMemo(() => {
    if (!originalDecision) { return false; }
    return result.decision !== originalDecision.decision
      || Math.abs(result.confidence - originalDecision.confidence) > 2
      || Math.abs(result.positionPct - originalDecision.positionPct) > 2;
  }, [result, originalDecision]);

  // Accept: save experiment record
  const handleAccept = useCallback(() => {
    const record: ExperimentRecord = {
      id: `${Date.now()}`,
      step: experiments.length + 1,
      params: { ...params },
      configOverrides: { ...configOverrides },
      decisionBefore: decision
        ? {
          action: decision.action,
          confidence: decision.confidence,
          positionPct: decision.positionPct,
          riskLevel: decision.riskLevel,
        }
        : {},
      decisionAfter: {
        action: result.decision as any,
        confidence: result.confidence,
        positionPct: result.positionPct,
        riskLevel: parseRiskLevel(result.riskLevel),
      },
      accepted: true,
      createdAt: Date.now(),
    };
    pushExperiment(record);
  }, [params, configOverrides, result, decision, experiments.length, pushExperiment]);

  // Apply config overrides to backend
  const handleApplyConfig = useCallback(async () => {
    if (!stockCode || Object.keys(configOverrides).length === 0) { return; }
    setToolReplayLoading(true);
    try {
      const res = await invoke<any>("replay_tool_chain", {
        params: { stockCode, configOverrides },
      });
      if (res?.totalScore != null) {
        setParams((p) => ({ ...p, totalScore: res.totalScore }));
      }
    } catch (e) {
      console.error("Tool chain replay failed:", e);
    }
    setToolReplayLoading(false);
  }, [stockCode, configOverrides]);

  // Reset params to original
  const handleReset = useCallback(() => {
    setParams(DEFAULT_PARAMS);
    setConfigOverrides({});
    if (decision) {
      setParams((p) => ({
        ...p,
        totalScore: typeof decision.confidence === "number" ? decision.confidence : 50,
        overallRisk: decision.riskLevel || "中",
      }));
    }
  }, [decision]);

  // Compute diff
  const diffItems = useMemo(() => {
    if (!originalDecision) { return []; }
    return [
      { label: "Decision", before: originalDecision.decision, after: result.decision },
      { label: "Confidence", before: `${originalDecision.confidence}%`, after: `${result.confidence}%` },
      { label: "Position", before: `${originalDecision.positionPct}%`, after: `${result.positionPct}%` },
    ];
  }, [originalDecision, result]);

  return (
    <div
      style={{
        border: "0.5px solid var(--color-border-info)",
        borderRadius: 8,
        overflow: "hidden",
        fontSize: 12,
      }}
    >
      {/* Header */}
      <div
        style={{
          background: "var(--color-background-info)",
          padding: "10px 12px",
          color: "var(--color-text-info)",
          fontWeight: 500,
          fontSize: 13,
        }}
      >
        Experiment
        <div style={{ fontSize: 11, fontWeight: 400, opacity: 0.7, marginTop: 2 }}>
          Modify params &middot; preview changes &middot; accept
        </div>
      </div>

      {/* Body */}
      <div style={{ padding: "10px 12px" }}>
        {/* What-If params */}
        <div style={{ marginBottom: 10 }}>
          <div style={{ fontWeight: 500, marginBottom: 6, fontSize: 11 }}>What-If parameters</div>
          <ParamSlider
            label="consensusScore"
            value={params.consensusScore}
            min={0}
            max={100}
            onChange={(v) => setParams((p) => ({ ...p, consensusScore: v }))}
          />
          <ParamSlider
            label="totalScore"
            value={params.totalScore}
            min={0}
            max={100}
            onChange={(v) => setParams((p) => ({ ...p, totalScore: v }))}
          />
          <ParamSlider
            label="dqiScore"
            value={params.dqiScore}
            min={0}
            max={100}
            onChange={(v) => setParams((p) => ({ ...p, dqiScore: v }))}
          />
          <ParamSelect
            label="overallRisk"
            value={params.overallRisk}
            options={["低", "中", "高", "极高"]}
            onChange={(v) => setParams((p) => ({ ...p, overallRisk: v }))}
          />
          <ParamSelect
            label="catalystLevel"
            value={params.catalystLevel}
            options={["无催化剂", "L1普通消息", "L2业绩拐点级", "L3估值体系级"]}
            onChange={(v) => setParams((p) => ({ ...p, catalystLevel: v }))}
          />
          <ParamSelect
            label="institutionalTrace"
            value={params.institutionalTrace}
            options={["无异常", "疑似建仓", "有建仓痕迹", "资金出逃"]}
            onChange={(v) => setParams((p) => ({ ...p, institutionalTrace: v }))}
          />
        </div>

        {/* Config overrides toggle */}
        <div style={{ borderTop: "0.5px solid var(--color-border-tertiary)", paddingTop: 8, marginBottom: 10 }}>
          <div
            onClick={() => setShowConfig(!showConfig)}
            style={{ cursor: "pointer", fontWeight: 500, fontSize: 11, marginBottom: showConfig ? 6 : 0 }}
          >
            {showConfig ? "▾" : "▸"} Config overrides
          </div>
          {showConfig && (
            <div>
              <ParamSlider
                label="scoring_trend"
                value={configOverrides.scoring_trend ?? 30}
                min={0}
                max={100}
                onChange={(v) => setConfigOverrides((p) => ({ ...p, scoring_trend: v }))}
              />
              <ParamSlider
                label="scoring_deviation"
                value={configOverrides.scoring_deviation ?? 20}
                min={0}
                max={100}
                onChange={(v) => setConfigOverrides((p) => ({ ...p, scoring_deviation: v }))}
              />
              <ParamSlider
                label="kelly_fraction"
                value={configOverrides.kelly_fraction ?? 0.5}
                min={0}
                max={1}
                step={0.05}
                onChange={(v) => setConfigOverrides((p) => ({ ...p, kelly_fraction: v }))}
              />
              <div style={{ marginTop: 4, display: "flex", justifyContent: "flex-end" }}>
                <Button size="small" loading={toolReplayLoading} onClick={handleApplyConfig}>
                  Apply
                </Button>
              </div>
            </div>
          )}
        </div>

        {/* Live preview */}
        <div style={{ borderTop: "0.5px solid var(--color-border-tertiary)", paddingTop: 8, marginBottom: 10 }}>
          <div style={{ fontSize: 11, color: "var(--color-text-secondary)", marginBottom: 4 }}>Preview</div>
          <div style={{ border: "0.5px solid var(--color-border-success)", borderRadius: 6, padding: "8px 10px" }}>
            <div style={{ display: "flex", gap: 16 }}>
              <div>
                <div style={{ fontSize: 10, color: "var(--color-text-secondary)" }}>Original</div>
                {diffItems.map((item) => (
                  <div key={item.label} style={{ fontSize: 11, marginTop: 2, color: "var(--color-text-secondary)" }}>
                    {item.before}
                  </div>
                ))}
              </div>
              <div>
                <div style={{ fontSize: 10, color: "var(--color-text-secondary)" }}>
                  {hasChanges ? "Modified" : "Same"}
                </div>
                <div
                  style={{
                    fontSize: 13,
                    fontWeight: 500,
                    color: hasChanges ? "var(--color-text-success)" : "var(--color-text-secondary)",
                    marginTop: 2,
                  }}
                >
                  {result.decision === "buy" ? "买入" : result.decision === "sell" ? "卖出" : "持有"}
                </div>
                <div style={{ fontSize: 11, color: "var(--color-text-secondary)" }}>
                  conf {result.confidence}% &middot; pos {result.positionPct}%
                </div>
              </div>
            </div>
          </div>
        </div>

        {/* Actions */}
        <div style={{ display: "flex", gap: 6 }}>
          <Button size="small" type="primary" disabled={!hasChanges} onClick={handleAccept} style={{ flex: 1 }}>
            Accept
          </Button>
          <Button size="small" onClick={handleReset} style={{ flex: 1 }}>
            Reset
          </Button>
        </div>
      </div>
    </div>
  );
}

// ── 子组件 ──

function ParamSlider({ label, value, min, max, step, onChange }: {
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  onChange: (v: number) => void;
}) {
  return (
    <div style={{ marginBottom: 6 }}>
      <div
        style={{
          display: "flex",
          justifyContent: "space-between",
          fontSize: 11,
          color: "var(--color-text-secondary)",
          marginBottom: 2,
        }}
      >
        <span>{label}</span>
        <span style={{ color: "var(--color-text-primary)" }}>{value}</span>
      </div>
      <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
        <Slider
          className="flex-1 !mb-0"
          min={min}
          max={max}
          step={step ?? 1}
          value={value}
          onChange={onChange}
        />
        <InputNumber
          className="!w-14 !text-[11px]"
          size="small"
          min={min}
          max={max}
          step={step ?? 1}
          value={value}
          onChange={(v) => onChange(v ?? min)}
          controls={false}
        />
      </div>
    </div>
  );
}

function ParamSelect({ label, value, options, onChange }: {
  label: string;
  value: string;
  options: string[];
  onChange: (v: string) => void;
}) {
  return (
    <div style={{ marginBottom: 6 }}>
      <div style={{ fontSize: 11, color: "var(--color-text-secondary)", marginBottom: 2 }}>{label}</div>
      <Select
        className="w-full"
        size="small"
        value={value}
        onChange={onChange}
        options={options.map((o) => ({ value: o, label: o }))}
      />
    </div>
  );
}
