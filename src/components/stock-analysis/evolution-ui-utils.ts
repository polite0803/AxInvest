/**
 * P4 进化审批 UI 共享工具：EvolutionDriftPanel 与 EvolutionApprovalDrawer 双入口复用。
 *
 * - parseTrigger：后端 trigger JSON（tagged enum, snake_case）→ 人类可读原因
 * - statusMeta：stock_evolution_history.status → Tag 颜色 + i18n key
 * - parseTriggerSamples / parseEvolutionResult：落库 JSON 的容错解析
 */

import type { EvolutionTriggerSamples } from "@/stores/feature/stockAnalysisStore";

type TFn = (key: string, params?: Record<string, unknown>) => string;

/** 毫秒时间戳 → 本地化字符串（0/空显示 —） */
export function formatEvolutionTime(ms: number | null | undefined): string {
  if (!ms) {
    return "—";
  }
  return new Date(ms).toLocaleString();
}

/** 把后端落库的 trigger JSON（tagged enum，snake_case）解析成人类可读的原因 */
export function parseTrigger(raw: string, t: TFn): string {
  if (!raw) {
    return "";
  }
  try {
    const obj = JSON.parse(raw) as Record<string, unknown>;
    switch (obj.type) {
      case "low_quality":
        return t("stockAnalysis.evolutionDrift.trigger.low_quality", {
          score: obj.last_score,
          count: obj.consecutive_count,
          threshold: obj.threshold,
        });
      case "poor_signal_accuracy":
        return t("stockAnalysis.evolutionDrift.trigger.poor_signal_accuracy", {
          score: obj.score,
        });
      case "missing_risk_assessment":
        return t("stockAnalysis.evolutionDrift.trigger.missing_risk_assessment");
      case "high_error_rate":
        return t("stockAnalysis.evolutionDrift.trigger.high_error_rate", {
          count: obj.error_count,
        });
      case "manual_trigger":
        return t("stockAnalysis.evolutionDrift.trigger.manual_trigger", {
          reason: typeof obj.reason === "string" ? obj.reason : "",
        });
      default:
        return raw;
    }
  } catch {
    return raw;
  }
}

/** 历史行状态 → antd Tag 颜色 + 文案 i18n key（文案在 stockAnalysis.evolutionApproval.status.*） */
export function evolutionStatusMeta(status: string): { color: string; textKey: string } {
  switch (status) {
    case "pending_approval":
      return { color: "orange", textKey: "stockAnalysis.evolutionApproval.status.pendingApproval" };
    case "accepted":
      return { color: "green", textKey: "stockAnalysis.evolutionApproval.status.accepted" };
    case "rejected":
      return { color: "red", textKey: "stockAnalysis.evolutionApproval.status.rejected" };
    // 历史只读状态（P4 前的存量行）
    case "success":
      return { color: "green", textKey: "stockAnalysis.evolutionApproval.status.success" };
    case "failed":
      return { color: "red", textKey: "stockAnalysis.evolutionApproval.status.failed" };
    default:
      return { color: "default", textKey: "stockAnalysis.evolutionApproval.status.unknown" };
  }
}

/** 容错解析触发样本快照；空/坏 JSON 返回 null */
export function parseTriggerSamples(raw: string | null | undefined): EvolutionTriggerSamples | null {
  if (!raw) {
    return null;
  }
  try {
    const obj = JSON.parse(raw) as Partial<EvolutionTriggerSamples>;
    return {
      reflections: Array.isArray(obj.reflections) ? obj.reflections : [],
      validations: Array.isArray(obj.validations) ? obj.validations : [],
      moverEvents: Array.isArray(obj.moverEvents) ? obj.moverEvents : [],
      blockedReflections: Array.isArray(obj.blockedReflections)
        ? obj.blockedReflections
        : [],
      blockedValidations: Array.isArray(obj.blockedValidations) ? obj.blockedValidations : [],
      blockedMoverEvents: Array.isArray(obj.blockedMoverEvents) ? obj.blockedMoverEvents : [],
    };
  } catch {
    return null;
  }
}

/** 权重衰减参数（对应后端 WeightDecayConfig，camelCase） */
export interface EvolutionWeightConfigView {
  lookbackDays: number;
  ewmaAlpha: number;
  sampleSaturation: number;
}

/** parameter_result_json 反序列化的最小前端视图（EvolutionResult） */
export interface EvolutionResultView {
  bestConfig: EvolutionWeightConfigView;
  defaultConfig: EvolutionWeightConfigView;
  /** 后端 evolutionStats 结构随优化器演进而变化，这里只透传不约束 */
  evolutionStats?: Record<string, unknown> | null;
}

/** 容错解析回测参数结果；空/坏 JSON 返回 null */
export function parseEvolutionResult(
  raw: string | null | undefined,
): EvolutionResultView | null {
  if (!raw) {
    return null;
  }
  try {
    const obj = JSON.parse(raw) as Partial<EvolutionResultView>;
    if (!obj.bestConfig || !obj.defaultConfig) {
      return null;
    }
    return {
      bestConfig: obj.bestConfig,
      defaultConfig: obj.defaultConfig,
      evolutionStats: obj.evolutionStats ?? null,
    };
  } catch {
    return null;
  }
}
