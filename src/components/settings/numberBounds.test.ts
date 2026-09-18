// SPDX-License-Identifier: AGPL-3.0-only
import { inferNumberBounds } from "@/components/settings/numberBounds";
import type { Variable } from "@/components/workflow/types";
import { describe, expect, it } from "vitest";

function num(name: string, value: unknown): Variable {
  return { name, varType: "number", value, isSecret: false };
}

/**
 * 用真实参数样本值覆盖 inferNumberBounds 的每一条分支。
 * 这些值取自 `seed_variables.rs`（stock-analysis 模板），不是编造的数。
 */
const SAMPLES: { name: string; value: number }[] = [
  // ① 温度类特例
  { name: "agent_temperature", value: 0.3 },
  // ② 负值（跨 0 才能表达）
  { name: "cap_kline_mom_5_min", value: -0.02 },
  { name: "earnings_th_strong_neg", value: -20 },
  { name: "earnings_th_huge_neg", value: -50 },
  // ③ 默认值 < 1 的比值 / 权重
  { name: "cost_pct", value: 0.003 },
  { name: "trend_high_20_threshold", value: 0.97 },
  { name: "cap_long_nb_ratio_min", value: 0.1 },
  { name: "cap_conf_market", value: 0 },
  // ④ 计数类
  { name: "keylevel_lookback_days", value: 60 },
  { name: "boll_period", value: 20 },
  { name: "scan_retry_max", value: 3 },
  { name: "keylevel_min_touches", value: 2 },
  // ⑤ 1~3 的乘数
  { name: "cap_ultra_short_entry_high", value: 1.005 },
  { name: "cap_long_stop", value: 0.88 },
  { name: "cap_long_target", value: 1.3 },
  { name: "boll_stddev", value: 2 },
  // ⑥ 其余 0~100 口径
  { name: "min_confidence", value: 60 },
  { name: "value_dcf_discount_rate", value: 8.5 },
  { name: "scoring_trend", value: 30 },
  // ② 默认值 > 100
  { name: "kline_limit", value: 120 },
  { name: "agent_timeout_secs", value: 600 },
  { name: "agent_max_tokens", value: 32768 },
  { name: "budget_high_max", value: 100000 },
];

describe("inferNumberBounds", () => {
  it("按默认值量级分层（代表性样本）", () => {
    expect(inferNumberBounds(num("agent_temperature", 0.3))).toEqual({ min: 0, max: 2, step: 0.1 });
    // 0.003 这类「0.x 量级百分数」必须走 0.001 细步长，不能被当成 0~100 口径
    expect(inferNumberBounds(num("cost_pct", 0.003))).toEqual({ min: 0, max: 2, step: 0.001 });
    expect(inferNumberBounds(num("cap_ultra_short_entry_high", 1.005))).toEqual({
      min: 0,
      max: 10,
      step: 0.001,
    });
    expect(inferNumberBounds(num("min_confidence", 60))).toEqual({ min: 0, max: 100, step: 0.1 });
    // 负值必须跨 0
    expect(inferNumberBounds(num("cap_kline_mom_5_min", -0.02))).toEqual({
      min: -0.1,
      max: 0.1,
      step: 0.001,
    });
    // 现值已超 100：上限必须抬到现值之上
    expect(inferNumberBounds(num("kline_limit", 120))).toEqual({ min: 0, max: 180, step: 1 });
  });

  it("I1: 默认值必须落在 [min, max] 内（上限低于现值会让滑杆把参数改小）", () => {
    const bad = SAMPLES.filter(({ name, value }) => {
      const { min, max } = inferNumberBounds(num(name, value));
      return !(min <= value && value <= max);
    });
    expect(bad).toEqual([]);
  });

  it("I2: 默认值必须落在 step 网格上（否则显示与写回不一致）", () => {
    const bad = SAMPLES.filter(({ name, value }) => {
      const { min, step } = inferNumberBounds(num(name, value));
      const q = (value - min) / step;
      return Math.abs(q - Math.round(q)) > 1e-6;
    });
    expect(bad).toEqual([]);
  });

  it("滑杆量程覆盖不到的值仍可由 InputNumber 键入（函数只决定手感，不夹取值域）", () => {
    // value 超出推断量程时函数本身不报错、不夹取，由调用方 clamp 显示。
    expect(inferNumberBounds(num("some_ratio", 0.5)).max).toBe(2);
    expect(inferNumberBounds(num("some_ratio", 0.5)).step).toBe(0.001);
  });
});
