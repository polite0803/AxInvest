import {
  actionToDirection,
  deriveActionFromLlmDecision,
  directionToAction,
  getActionTKey,
  parseAction,
  parseActionStrict,
  parseDirectionFromText,
  parsePositionState,
  PositionState,
  resolveDisplayAction,
  StockAction,
} from "@/lib/stock-analysis-utils";
import { describe, expect, it } from "vitest";

// trader.md 第 93 行的示例 reasoning（prompt 自带文本，逐字引用）。
// 旧实现用 `raw.includes(label)` 按对象键序扫描：`减持`(键4) 先于 `看多`(键12)，
// 于是这段看多文本被解析成 REDUCE —— 出现风险词即方向反转。
const TRADER_MD_SAMPLE_REASONING =
  "方向:看多。综合 ① research-mgr Q2 营收+35% 成长性论证；② debate-convergence 共识 68 分且 R2 反驳未触及业绩核心；③ a-catalyst L2 业绩拐点级催化剂；④ t-scoring RSI=58 + MACD 金叉技术面支撑。风险点：大股东减持 5%（risk-convergence 三方分歧 45，接近阈值）。数据缺口：PE 缺失、龙虎榜无数据，置信度由 80 下调至 72%。";

describe("parseAction 严格值域解析", () => {
  it("英文枚举大小写/空白容错", () => {
    expect(parseAction("BUY")).toBe(StockAction.BUY);
    expect(parseAction("  sell  ")).toBe(StockAction.SELL);
    expect(parseAction("Hold")).toBe(StockAction.HOLD);
  });

  it("中文标签严格全等（不再靠 includes）", () => {
    expect(parseAction("买入")).toBe(StockAction.BUY);
    expect(parseAction("增持")).toBe(StockAction.INCREASE);
    expect(parseAction("观望")).toBe(StockAction.WAIT);
    expect(parseAction("减持")).toBe(StockAction.REDUCE);
    expect(parseAction("数据缺失")).toBe(StockAction.UNAVAILABLE);
  });

  it("未识别返回 UNCERTAIN，不再伪装成 WAIT", () => {
    expect(parseAction("")).toBe(StockAction.UNCERTAIN);
    expect(parseAction(null)).toBe(StockAction.UNCERTAIN);
    expect(parseAction(undefined)).toBe(StockAction.UNCERTAIN);
    expect(parseAction(42)).toBe(StockAction.UNCERTAIN);
    expect(parseAction("待明日观察量能变化")).toBe(StockAction.UNCERTAIN);
  });

  // 负向回归：这三条是本次修复的核心证据（旧实现 3/5 不符）
  it("自由文本不得被解析成 action（方向反转防回归）", () => {
    // 看多文本里含「大股东减持」→ 旧实现返回 REDUCE
    expect(parseAction(TRADER_MD_SAMPLE_REASONING)).toBe(StockAction.UNCERTAIN);
    // 看空文本里含「不建议买入」→ 旧实现返回 BUY
    expect(parseAction("方向:看空。行业景气度下行，不建议买入。")).toBe(StockAction.UNCERTAIN);
    // 中性文本里含「等待」→ 旧实现返回 WAIT（被无关词先命中）
    expect(parseAction("方向:中性。等待更清晰的信号。")).toBe(StockAction.UNCERTAIN);
  });

  it("verdict 词（看多/看空/中性）已退出 action 值域空间", () => {
    expect(parseActionStrict("看多")).toBeNull();
    expect(parseActionStrict("看空")).toBeNull();
    expect(parseActionStrict("中性")).toBeNull();
  });
});

describe("parseDirectionFromText 只认显式方向标记", () => {
  it("读取「方向:」标记且不被句中风险词干扰", () => {
    expect(parseDirectionFromText(TRADER_MD_SAMPLE_REASONING)).toBe("看多");
    expect(parseDirectionFromText("方向：看空。建议减仓。")).toBe("看空");
    expect(parseDirectionFromText("方向: 中性")).toBe("中性");
    expect(parseDirectionFromText('"verdict":"bearish"')).toBe("看空");
  });

  it("无显式标记返回 null（不臆造方向）", () => {
    expect(parseDirectionFromText("综合来看基本面改善，估值合理。")).toBeNull();
    expect(parseDirectionFromText("")).toBeNull();
    expect(parseDirectionFromText(null)).toBeNull();
  });
});

describe("directionToAction 降维映射（与 trader.md 一致性表对齐）", () => {
  it("三值方向 → 强度档默认值", () => {
    expect(directionToAction("看多")).toBe(StockAction.BUY);
    expect(directionToAction("看空")).toBe(StockAction.SELL);
    expect(directionToAction("中性")).toBe(StockAction.HOLD);
    expect(directionToAction("bullish")).toBe(StockAction.BUY);
    expect(directionToAction("")).toBeNull();
  });
});

describe("deriveActionFromLlmDecision 按结构化程度降序推导", () => {
  it("结构化 action 字段优先级最高", () => {
    expect(deriveActionFromLlmDecision({ action: "增持", verdict: "看空" })).toBe(StockAction.INCREASE);
  });

  it("无 action 时读 verdict", () => {
    expect(deriveActionFromLlmDecision({ verdict: "看空" })).toBe(StockAction.SELL);
    expect(deriveActionFromLlmDecision({ stance: "中性" })).toBe(StockAction.HOLD);
  });

  it("无结构化字段时读 reasoning 的显式方向标记", () => {
    expect(deriveActionFromLlmDecision({ reasoning: TRADER_MD_SAMPLE_REASONING })).toBe(StockAction.BUY);
  });

  it("全部不可用返回 null（调用方须保留原值，不得臆造）", () => {
    expect(deriveActionFromLlmDecision({ reasoning: "基本面稳健，估值合理。" })).toBeNull();
    expect(deriveActionFromLlmDecision({})).toBeNull();
    expect(deriveActionFromLlmDecision(null)).toBeNull();
  });
});

describe("actionToDirection 方向映射（下单表单）", () => {
  it("全 8 值覆盖：仅方向明确的档位有方向", () => {
    expect(actionToDirection(StockAction.BUY)).toBe("buy");
    expect(actionToDirection(StockAction.INCREASE)).toBe("buy");
    expect(actionToDirection("增持")).toBe("buy");
    expect(actionToDirection(StockAction.SELL)).toBe("sell");
    expect(actionToDirection(StockAction.REDUCE)).toBe("sell");
    expect(actionToDirection("减持")).toBe("sell");
    // 关键：以下档位**必须**为 null，否则会被当成买入填进下单表单
    expect(actionToDirection(StockAction.HOLD)).toBeNull();
    expect(actionToDirection(StockAction.WAIT)).toBeNull();
    expect(actionToDirection(StockAction.UNCERTAIN)).toBeNull();
    expect(actionToDirection(StockAction.UNAVAILABLE)).toBeNull();
    expect(actionToDirection("观望")).toBeNull();
    expect(actionToDirection(null)).toBeNull();
  });
});

describe("getActionTKey i18n 键", () => {
  it("UNAVAILABLE 与 UNCERTAIN 各有独立键", () => {
    expect(getActionTKey(StockAction.UNAVAILABLE)).toBe("stockAnalysis.actionUnavailable");
    expect(getActionTKey(StockAction.UNCERTAIN)).toBe("stockAnalysis.actionUncertain");
    expect(getActionTKey("观望")).toBe("stockAnalysis.actionWait");
    expect(getActionTKey("减持")).toBe("stockAnalysis.actionReduce");
  });
});

describe("resolveDisplayAction 持有/观望派生化（两轴正交）", () => {
  it("非中性档不受持仓状态影响", () => {
    expect(resolveDisplayAction("买入", "EMPTY", 0)).toBe(StockAction.BUY);
    expect(resolveDisplayAction("增持", "TRIMMING", 30)).toBe(StockAction.INCREASE);
    expect(resolveDisplayAction("减持", "EMPTY", 0)).toBe(StockAction.REDUCE);
    expect(resolveDisplayAction("数据缺失", "EMPTY", 0)).toBe(StockAction.UNAVAILABLE);
  });

  it("中性档按 positionState 派生（空仓→观望，有仓位→持有）", () => {
    expect(resolveDisplayAction(StockAction.HOLD, "EMPTY", 20)).toBe(StockAction.WAIT);
    expect(resolveDisplayAction(StockAction.WAIT, "HOLDING", 0)).toBe(StockAction.HOLD);
    expect(resolveDisplayAction(StockAction.HOLD, "OPENING", 5)).toBe(StockAction.HOLD);
    expect(resolveDisplayAction("观望", "TRIMMING", 5)).toBe(StockAction.HOLD);
  });

  it("positionState 缺失（老数据）退回 positionPct", () => {
    expect(resolveDisplayAction(StockAction.HOLD, null, 0)).toBe(StockAction.WAIT);
    expect(resolveDisplayAction(StockAction.WAIT, null, 12)).toBe(StockAction.HOLD);
    // 两者都不可用 → 保持原值，不臆造
    expect(resolveDisplayAction(StockAction.WAIT, undefined, null)).toBe(StockAction.WAIT);
    expect(resolveDisplayAction(StockAction.HOLD, undefined, undefined)).toBe(StockAction.HOLD);
  });

  it("positionState=null 不得被读成 EMPTY", () => {
    // 若把 null 当 EMPTY，本条会错误地派生成 WAIT（把「不知道」当成「空仓」）
    expect(resolveDisplayAction(StockAction.HOLD, null, 30)).toBe(StockAction.HOLD);
  });

  it("parsePositionState 严格值域", () => {
    expect(parsePositionState("holding")).toBe(PositionState.HOLDING);
    expect(parsePositionState(" EMPTY ")).toBe(PositionState.EMPTY);
    expect(parsePositionState("")).toBeNull();
    expect(parsePositionState(null)).toBeNull();
    expect(parsePositionState("空仓")).toBeNull();
  });
});

describe("后端移除「观望 ⇄ 持有」互改后的两轴契约", () => {
  it("空仓看多：action 保真为买入/增持，展示层不得改写回观望", () => {
    // 互改存在时后端会把这类记录落库成「观望」；移除后落库「买入/增持」+ positionState=EMPTY。
    // 展示层只对 HOLD/WAIT 派生 ⇒ 必须原样显示「买入」，
    // 否则前端会把后端刚拆开的两轴又焊回去（保真度白改）。
    expect(resolveDisplayAction("买入", "EMPTY", 0)).toBe(StockAction.BUY);
    expect(resolveDisplayAction("增持", "EMPTY", 0)).toBe(StockAction.INCREASE);
    // 对照：真正被判定为中性、或因风控/低置信降级成「持有」的，才按空仓派生成「观望」
    expect(resolveDisplayAction("持有", "EMPTY", 0)).toBe(StockAction.WAIT);
  });

  it("试探仓：action=观望 + 有仓位（互改时代会被升写成「持有」）", () => {
    // 移除后 action 保持「观望」（方向判断保真）、持仓由 positionState 表达；
    // 展示层派生为「持有」⇒ 用户看到的结果不变，但落库数据两轴可分别审计。
    expect(parsePositionState("HOLDING")).toBe(PositionState.HOLDING);
    expect(resolveDisplayAction("观望", "HOLDING", 3)).toBe(StockAction.HOLD);
  });

  it("合法组合「有方向 + 空仓」可表达（互改时代不可达）", () => {
    // 凯利/风控把目标仓位算成 0 时，方向判断仍然成立 —— 这在互改时代会被改写成「观望」而丢失。
    expect(resolveDisplayAction("买入", "EMPTY", 0)).toBe(StockAction.BUY);
    expect(resolveDisplayAction("减持", "EMPTY", 0)).toBe(StockAction.REDUCE);
    // 中性档才允许被持仓状态改写
    expect(resolveDisplayAction("观望", "EMPTY", 0)).toBe(StockAction.WAIT);
  });
});
