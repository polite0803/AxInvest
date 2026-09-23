import {
  actionToDirection,
  alignReasoningDecisionLabel,
  classifyDirectionText,
  classifySentiment,
  deriveActionFromLlmDecision,
  directionToAction,
  getActionTKey,
  getCatalystDirectionColor,
  getCatalystDirectionTKey,
  getCatalystTimelineTKey,
  getChecklistCategoryTKey,
  getDashboardActionColor,
  getDashboardActionTKey,
  getDashboardSeverityColor,
  getDashboardSeverityTKey,
  getDashboardTrendColor,
  getDashboardTrendTKey,
  parseAction,
  parseActionStrict,
  parseDirectionFromText,
  parsePositionState,
  PositionState,
  resolveDisplayAction,
  StockAction,
} from "@/lib/stock-analysis-utils";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
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

describe("resolveDisplayAction —— 展示档 = 方向档（2026-09-22 起不再按仓位派生）", () => {
  it("各档一律不受 positionState / positionPct 影响", () => {
    // 非中性档
    expect(resolveDisplayAction("买入", "EMPTY", 0)).toBe(StockAction.BUY);
    expect(resolveDisplayAction("增持", "TRIMMING", 30)).toBe(StockAction.INCREASE);
    expect(resolveDisplayAction("减持", "EMPTY", 0)).toBe(StockAction.REDUCE);
    expect(resolveDisplayAction("数据缺失", "EMPTY", 0)).toBe(StockAction.UNAVAILABLE);
    // 中性档：旧派生会按仓位翻名（HOLD+EMPTY→WAIT / WAIT+HOLDING→HOLD），现必须恒等
    expect(resolveDisplayAction(StockAction.HOLD, "EMPTY", 0)).toBe(StockAction.HOLD);
    expect(resolveDisplayAction(StockAction.HOLD, "EMPTY", 20)).toBe(StockAction.HOLD);
    expect(resolveDisplayAction(StockAction.HOLD, "HOLDING", 30)).toBe(StockAction.HOLD);
    expect(resolveDisplayAction(StockAction.WAIT, "HOLDING", 3)).toBe(StockAction.WAIT);
    expect(resolveDisplayAction(StockAction.WAIT, "OPENING", 5)).toBe(StockAction.WAIT);
  });

  it("回归 AUDIT-300642：同为「观望」档、仓位 0% vs 3% 必须给出同一展示档", () => {
    // 修复前：0% ⇒ positionState=EMPTY ⇒ 展示「观望」；3%（LLM 措辞放行的试探仓）
    //   ⇒ HOLDING ⇒ 展示「持有」。同一只股票同日两次跑出两个结论，即本次投诉。
    expect(resolveDisplayAction("观望", "EMPTY", 0)).toBe(StockAction.WAIT);
    expect(resolveDisplayAction("观望", "HOLDING", 3)).toBe(StockAction.WAIT);
  });

  it("positionState 缺失 / 非法 / 为 null 均不影响结果（判定已不依赖它）", () => {
    expect(resolveDisplayAction(StockAction.HOLD, null, 0)).toBe(StockAction.HOLD);
    expect(resolveDisplayAction(StockAction.WAIT, null, 12)).toBe(StockAction.WAIT);
    expect(resolveDisplayAction(StockAction.WAIT, undefined, null)).toBe(StockAction.WAIT);
    expect(resolveDisplayAction(StockAction.HOLD, undefined, undefined)).toBe(StockAction.HOLD);
  });

  it("parsePositionState 严格值域", () => {
    expect(parsePositionState("holding")).toBe(PositionState.HOLDING);
    expect(parsePositionState(" EMPTY ")).toBe(PositionState.EMPTY);
    expect(parsePositionState("")).toBeNull();
    expect(parsePositionState(null)).toBeNull();
    expect(parsePositionState("空仓")).toBeNull();
  });
});

describe("两轴契约：方向档保真 + 持仓状态独立表达", () => {
  it("空仓看多：action 保真为买入/增持，展示层不得改写回观望", () => {
    // 互改存在时后端会把这类记录落库成「观望」；移除后落库「买入/增持」+ positionState=EMPTY。
    // 展示层必须原样显示「买入」，否则前端会把后端刚拆开的两轴又焊回去（保真度白改）。
    expect(resolveDisplayAction("买入", "EMPTY", 0)).toBe(StockAction.BUY);
    expect(resolveDisplayAction("增持", "EMPTY", 0)).toBe(StockAction.INCREASE);
  });

  it("试探仓：action=观望 + 有仓位 —— 两轴各自保真，展示档仍为「观望」", () => {
    // ⚠️ 2026-09-22 语义变更：旧实现把展示档派生成「持有」（这就是循环判据的出口，
    //   让建议仓位反过来改结论名）。现仓位只由 positionState / positionPct 表达，
    //   方向档是什么就展示什么。
    expect(parsePositionState("HOLDING")).toBe(PositionState.HOLDING);
    expect(resolveDisplayAction("观望", "HOLDING", 3)).toBe(StockAction.WAIT);
  });

  it("合法组合「有方向 + 空仓」可表达（互改时代不可达）", () => {
    // 凯利/风控把目标仓位算成 0 时，方向判断仍然成立 —— 这在互改时代会被改写成「观望」而丢失。
    expect(resolveDisplayAction("买入", "EMPTY", 0)).toBe(StockAction.BUY);
    expect(resolveDisplayAction("减持", "EMPTY", 0)).toBe(StockAction.REDUCE);
    expect(resolveDisplayAction("观望", "EMPTY", 0)).toBe(StockAction.WAIT);
  });
});

describe("alignReasoningDecisionLabel —— reasoning 结论名对齐最终方向档", () => {
  // 实证样本（2026-09-21 13:52 的真实落库记录）：reasoning 文本写「决策=持有」，
  // 而最终方向档是「观望」—— 同一条记录内部两个名字，用户直接质问矛盾。
  // 2026-09-22 起展示档不再按仓位派生（= 方向档恒等）；本函数兜的剩余场景是
  // **跨节点不同步**：`portfolio-risk-gate.rhai` 覆盖 action 但不重写 reasoning。
  const SAMPLE = "决策=持有 置信=35.9 仓位=0% | 先验=0.5 后验=0.51 | ⚠️双视角部分一致:58分";

  it("跨节点不同步时，开头的结论名对齐最终档（风控门改档样本）", () => {
    expect(alignReasoningDecisionLabel(SAMPLE, StockAction.WAIT)).toBe(
      "决策=观望 置信=35.9 仓位=0% | 先验=0.5 后验=0.51 | ⚠️双视角部分一致:58分",
    );
  });

  it("挂角与结论名同源（端到端一致性）", () => {
    // 展示档 = 最终方向档；两条展示路径（挂角 Tag / reasoning 前缀）必须同源
    const display = resolveDisplayAction(StockAction.WAIT);
    expect(getActionTKey(display)).toBe("stockAnalysis.actionWait");
    expect(alignReasoningDecisionLabel(SAMPLE, display).startsWith("决策=观望")).toBe(true);
  });

  it("幂等：已一致时逐字节不变", () => {
    const once = alignReasoningDecisionLabel(SAMPLE, StockAction.WAIT);
    expect(alignReasoningDecisionLabel(once, StockAction.WAIT)).toBe(once);
    expect(alignReasoningDecisionLabel(SAMPLE, StockAction.HOLD)).toBe(SAMPLE);
  });

  it("只改开头第一个 `决策=X`，过程留痕（档位迁移）保持原样", () => {
    // `⚠️空头预测否决:持有→卖出` 记的是**迁移过程**，用原始档名才是对的
    const text = "决策=持有 置信=40 仓位=3% | ⚠️空头预测否决:持有→卖出";
    expect(alignReasoningDecisionLabel(text, StockAction.WAIT)).toBe(
      "决策=观望 置信=40 仓位=3% | ⚠️空头预测否决:持有→卖出",
    );
  });

  it("以 displayAction 为唯一真相源：下游改档时结论名跟随最终档", () => {
    // `portfolio-risk-gate.rhai` 覆盖 action 时**不更新** reasoning 的结论名
    // （只追加 `| [风控门] ...`）⇒ 文本停留在 pm 的档位，此处按最终展示档对齐。
    const text = "决策=持有 置信=40 仓位=5% | [风控门] 高风险禁止加仓";
    expect(alignReasoningDecisionLabel(text, StockAction.WAIT)).toBe(
      "决策=观望 置信=40 仓位=5% | [风控门] 高风险禁止加仓",
    );
  });

  it("无前缀 / 档名识别不出 / 空串 ⇒ 原样返回，不臆造结论", () => {
    // LLM 侧 reasoning、其它来源文本不带本前缀
    expect(alignReasoningDecisionLabel("基本面稳健，估值合理。", StockAction.WAIT)).toBe(
      "基本面稳健，估值合理。",
    );
    // 档名识别不出 ⇒ 改写等于把「看不懂」当成「已确认」
    expect(alignReasoningDecisionLabel("决策=说不清 置信=30", StockAction.WAIT)).toBe(
      "决策=说不清 置信=30",
    );
    expect(alignReasoningDecisionLabel("", StockAction.WAIT)).toBe("");
  });
});

// ── `dashboard_report` 值域（2026-09-21）──
// 背景：`DashboardReportPreview` 长期把 action / trend / severity / category /
// direction+timeline 五处**裸渲染**，靠文件头一行 `i18n-exempt` 让硬编码扫描器跳过
// 整个文件 —— zh-CN 下看不出问题，切到 en-US 等语言整片中文乱入。修复把五个值域
// 收敛到 `stock-analysis-utils` 单点，本块钉住该单点的行为。
describe("dashboard_report 值域 → i18n key", () => {
  it("「强烈买入 / 强烈卖出」必须走 dashboard 专属 key，不得被收敛成 BUY / SELL", () => {
    expect(getDashboardActionTKey("强烈买入")).toBe("stockAnalysis.dashboard.actionStrongBuy");
    expect(getDashboardActionTKey("强烈卖出")).toBe("stockAnalysis.dashboard.actionStrongSell");
    // ⚠ 变异检验：删掉 `DASHBOARD_ACTION_TKEY_OVERRIDES` 后，`parseActionStrict` 会把
    //    这两档收敛成 BUY / SELL ⇒ 界面把「强烈买入」显示成「买入」（丢失强度）。
    //    仅断言「等于专属 key」不足以钉住该行为，须显式断言「不等于收敛结果」——
    //    否则两张表合并后本测试仍然全绿，而缺陷已经回来了。
    expect(getDashboardActionTKey("强烈买入")).not.toBe("stockAnalysis.actionBuy");
    expect(getDashboardActionTKey("强烈卖出")).not.toBe("stockAnalysis.actionSell");
  });

  it("非强度档复用顶层 actionXxx key（同一批文案，不另存一份翻译）", () => {
    expect(getDashboardActionTKey("买入")).toBe("stockAnalysis.actionBuy");
    expect(getDashboardActionTKey("增持")).toBe("stockAnalysis.actionIncrease");
    expect(getDashboardActionTKey("持有")).toBe("stockAnalysis.actionHold");
    expect(getDashboardActionTKey("减持")).toBe("stockAnalysis.actionReduce");
    expect(getDashboardActionTKey("卖出")).toBe("stockAnalysis.actionSell");
  });

  it("解析不出 / 空 / 非字符串 ⇒ 权威哨兵，不臆造档位", () => {
    expect(getDashboardActionTKey("说不清的档位")).toBe("stockAnalysis.actionUncertain");
    expect(getDashboardActionTKey("")).toBe("stockAnalysis.actionUnavailable");
    expect(getDashboardActionTKey("   ")).toBe("stockAnalysis.actionUnavailable");
    expect(getDashboardActionTKey(null)).toBe("stockAnalysis.actionUnavailable");
    expect(getDashboardActionTKey(undefined)).toBe("stockAnalysis.actionUnavailable");
  });

  it("trend 值域是「看多 / 看空 / 震荡」—— 不是 verdict 空间的「中性」", () => {
    expect(getDashboardTrendTKey("看多")).toBe("stockAnalysis.dashboard.trendBullish");
    expect(getDashboardTrendTKey("看空")).toBe("stockAnalysis.dashboard.trendBearish");
    expect(getDashboardTrendTKey("震荡")).toBe("stockAnalysis.dashboard.trendSideways");
    // 方向词属 verdict 空间，混入会被静默吞掉（见 STOCK_ACTION_LABELS 顶部注释）
    expect(getDashboardTrendTKey("中性")).toBeNull();
  });

  it("severity / checklist category / catalyst direction / timeline 各值域", () => {
    expect(getDashboardSeverityTKey("高")).toBe("stockAnalysis.dashboard.severityHigh");
    expect(getDashboardSeverityTKey("中")).toBe("stockAnalysis.dashboard.severityMid");
    expect(getDashboardSeverityTKey("低")).toBe("stockAnalysis.dashboard.severityLow");
    // riskLevel 是另一套措辞（低风险/中风险/…），不得混入 severity 值域
    expect(getDashboardSeverityTKey("极高")).toBeNull();

    expect(getChecklistCategoryTKey("入场")).toBe("stockAnalysis.dashboard.checklistEntry");
    expect(getChecklistCategoryTKey("加仓")).toBe("stockAnalysis.dashboard.checklistAdd");
    expect(getChecklistCategoryTKey("减仓")).toBe("stockAnalysis.dashboard.checklistReduce");
    expect(getChecklistCategoryTKey("止损")).toBe("stockAnalysis.dashboard.checklistStopLoss");
    expect(getChecklistCategoryTKey("止盈")).toBe("stockAnalysis.dashboard.checklistTakeProfit");
    expect(getChecklistCategoryTKey("其它类别")).toBeNull();

    expect(getCatalystDirectionTKey("利好")).toBe("stockAnalysis.dashboard.catalystBullish");
    expect(getCatalystDirectionTKey("利空")).toBe("stockAnalysis.dashboard.catalystBearish");

    expect(getCatalystTimelineTKey("短期")).toBe("stockAnalysis.dashboard.catalystShortTerm");
    expect(getCatalystTimelineTKey("中期")).toBe("stockAnalysis.dashboard.catalystMidTerm");
    expect(getCatalystTimelineTKey("长期")).toBe("stockAnalysis.dashboard.catalystLongTerm");
    expect(getCatalystTimelineTKey(null)).toBeNull();
  });

  it("action 配色保留强度渐变，不得与 getActionColor 的方向二值色合并", () => {
    expect(getDashboardActionColor("强烈买入")).toBe("#f5222d");
    expect(getDashboardActionColor("买入")).toBe("#fa541c");
    expect(getDashboardActionColor("增持")).toBe("#fa8c16");
    expect(getDashboardActionColor("持有")).toBe("#8c8c8c");
    expect(getDashboardActionColor("减持")).toBe("#52c41a");
    expect(getDashboardActionColor("卖出")).toBe("#13c2c2");
    // 强度档三色互不相同 —— 若「强度渐变」被拍平成方向二值色，本条立即失败
    const ramp = ["强烈买入", "买入", "增持"].map((v) => getDashboardActionColor(v));
    expect(new Set(ramp).size).toBe(3);
    // 未知值取中性灰，不臆造方向色
    expect(getDashboardActionColor("未知档")).toBe("#8c8c8c");
  });

  it("配色遵循 A 股涨跌习惯：看多红 / 看空绿 / 利好红 / 利空绿", () => {
    expect(getDashboardTrendColor("看多")).toBe("#f5222d");
    expect(getDashboardTrendColor("看空")).toBe("#52c41a");
    expect(getDashboardTrendColor("震荡")).toBe("#8c8c8c");
    expect(getCatalystDirectionColor("利好")).toBe("red");
    expect(getCatalystDirectionColor("利空")).toBe("green");
    expect(getDashboardSeverityColor("高")).toBe("red");
    expect(getDashboardSeverityColor("中")).toBe("orange");
    expect(getDashboardSeverityColor("低")).toBe("green");
    expect(getDashboardSeverityColor("极高")).toBe("default");
  });
});

// ⚠ 本块检验的是**跨文件契约**：函数返回的 key 若在 locale 里不存在，界面会直接
//    渲染出 key 字符串本身（react-i18next 未命中时返回 key 原文）。这与
//    「前端 map 拼字符串 ↔ 后端 format! 产出」的幽灵键属同型缺陷，靠值域单测
//    是**查不出来**的 —— 必须跨到 locale 文件上做存在性断言。
describe("dashboard 值域返回的 i18n key 必须在 11 个 locale 中真实存在", () => {
  const LANGS = ["zh-CN", "zh-TW", "en-US", "ja", "ko", "de", "fr", "es", "ru", "ar", "hi"];
  const LOCALES_DIR = path.resolve(
    path.dirname(fileURLToPath(import.meta.url)),
    "..",
    "..",
    "i18n",
    "locales",
  );

  /** 穷举全部值域的**合法输入**，用函数自身产出 key 集合（不手工列举，避免漏同步） */
  const probes: Array<() => string | null> = [
    ...["强烈买入", "强烈卖出", "买入", "增持", "持有", "减持", "卖出", "观望", "不确定", "数据缺失"]
      .map((v) => () => getDashboardActionTKey(v)),
    ...["看多", "看空", "震荡"].map((v) => () => getDashboardTrendTKey(v)),
    ...["低", "中", "高"].map((v) => () => getDashboardSeverityTKey(v)),
    ...["入场", "加仓", "减仓", "止损", "止盈"].map((v) => () => getChecklistCategoryTKey(v)),
    ...["利好", "利空"].map((v) => () => getCatalystDirectionTKey(v)),
    ...["短期", "中期", "长期"].map((v) => () => getCatalystTimelineTKey(v)),
  ];

  it("值域产出的 key 集合非空，且每个都在 11 语言中有非空字符串值", () => {
    const keys = [...new Set(probes.map((f) => f()).filter((k): k is string => k !== null))];
    // 防假绿：若 probes 退化成空集合，下面的双层循环一次都不执行 ⇒ 测试恒真
    expect(keys.length).toBe(26);

    for (const lang of LANGS) {
      const dict = JSON.parse(
        fs.readFileSync(path.join(LOCALES_DIR, `${lang}.json`), "utf8"),
      ) as Record<string, unknown>;
      for (const key of keys) {
        let cur: unknown = dict;
        for (const seg of key.split(".")) {
          cur = cur && typeof cur === "object" ? (cur as Record<string, unknown>)[seg] : undefined;
        }
        // 缺 key / 空串 / 非字符串全部判失败
        expect(typeof cur === "string" && cur.trim() !== "", `${lang} 缺 key: ${key}`).toBe(true);
      }
    }
  });

  it("未知值返回 null（由调用方展示原文），而非臆造一个 key", () => {
    expect(getDashboardTrendTKey("不存在的趋势")).toBeNull();
    expect(getDashboardSeverityTKey("不存在的等级")).toBeNull();
    expect(getChecklistCategoryTKey("不存在的类别")).toBeNull();
    expect(getCatalystDirectionTKey("不存在的方向")).toBeNull();
    expect(getCatalystTimelineTKey("不存在的时间线")).toBeNull();
  });
});

/**
 * 2026-09-21 回归：多空方向判据此前在 4 处各写一份、值域各不相同
 * （AnalystReportGrid ×2、AnalystReportCard ×2），导致同一份 `verdict: "买入"`
 * 的研报在 Grid 判为看多（红）、在 Card 判不出方向（灰/中性）。
 * 现收敛到 `classifyDirectionText`，其值域是四份旧表的**并集**。
 */
describe("classifyDirectionText —— 多空方向单一真相源", () => {
  it("四份旧值域的并集全部识别（这是收敛的关键：任一旧表认得的值新表都必须认得）", () => {
    // Grid「多空分数兜底」独有
    for (const w of ["看多", "买入", "增持", "做多", "看涨", "bull"]) {
      expect(classifyDirectionText(w), `bull 应识别: ${w}`).toBe("bull");
    }
    for (const w of ["看空", "卖出", "减持", "做空", "看跌", "bear"]) {
      expect(classifyDirectionText(w), `bear 应识别: ${w}`).toBe("bear");
    }
    // Grid「判断」列独有
    for (const w of ["偏多", "正面"]) {
      expect(classifyDirectionText(w), `bull 应识别: ${w}`).toBe("bull");
    }
    for (const w of ["偏空", "负面"]) {
      expect(classifyDirectionText(w), `bear 应识别: ${w}`).toBe("bear");
    }
  });

  it("英文大小写 / 前后空白不敏感", () => {
    expect(classifyDirectionText("  Bullish  ")).toBe("bull");
    expect(classifyDirectionText("BEARISH")).toBe("bear");
  });

  it("无法判定返回 null（不臆造方向）", () => {
    expect(classifyDirectionText("中性")).toBeNull();
    expect(classifyDirectionText("")).toBeNull();
    expect(classifyDirectionText("   ")).toBeNull();
    expect(classifyDirectionText(null)).toBeNull();
    expect(classifyDirectionText(undefined)).toBeNull();
    expect(classifyDirectionText(42)).toBeNull();
    expect(classifyDirectionText({ verdict: "看多" })).toBeNull();
  });

  it("判定顺序显式固定为「先多后空」（两词同现时判为看多）", () => {
    // ⚠️ 这是四份旧实现的既有取舍，此处显式固定以防被无意改序。
    //   「看多…但需注意减持风险」这类两词同现判为看多，属已知的粗糙处；
    //   需要更精细的双向判定应另设计判据，不要在此函数上叠分支。
    expect(classifyDirectionText("看多，但需注意减持风险")).toBe("bull");
    // 只含空头词时为看空
    expect(classifyDirectionText("看空，且资金持续流出")).toBe("bear");
  });
});

const VERDICT_BRANCH = (verdict: string) => `报告正文\n<!-- VERDICT: ${JSON.stringify({ verdict })} -->`;

/**
 * 2026-09-21 回归：`classifySentiment` 内部此前有**两份**内联方向词表
 * （VERDICT 分支 / stance 分支），值与 4 个组件站点的表又各不同。
 * 现两条分支共用 `directionTextToSentiment`（方向词走 `classifyDirectionText`，
 * 情绪词在共用函数里拼接）⇒ 同一文本的结论必须一致。
 */
describe("classifySentiment 两分支结论一致（防三份内联表回流）", () => {
  const BULL_WORDS = ["偏多", "正面", "多头", "利好", "超配", "扫货", "买入", "看多"];
  const BEAR_WORDS = ["偏空", "负面", "空头", "利空", "低配", "出货", "卖出", "看空"];

  it("两分支对同一多头词都给 bullish", () => {
    for (const w of BULL_WORDS) {
      expect(classifySentiment(JSON.stringify({ stance: w })), `stance 分支: ${w}`).toBe("bullish");
      expect(classifySentiment(VERDICT_BRANCH(w)), `VERDICT 分支: ${w}`).toBe("bullish");
    }
  });

  it("两分支对同一空头词都给 bearish", () => {
    for (const w of BEAR_WORDS) {
      expect(classifySentiment(JSON.stringify({ stance: w })), `stance 分支: ${w}`).toBe("bearish");
      expect(classifySentiment(VERDICT_BRANCH(w)), `VERDICT 分支: ${w}`).toBe("bearish");
    }
  });

  it("两分支的中性词表并集都生效（平衡/同步/保守/放缓 原先只在一份里）", () => {
    for (const w of ["中性", "观望", "持有", "震荡", "平衡", "同步", "保守", "放缓"]) {
      expect(classifySentiment(JSON.stringify({ stance: w })), `stance 分支中性: ${w}`).toBe("neutral");
      expect(classifySentiment(VERDICT_BRANCH(w)), `VERDICT 分支中性: ${w}`).toBe("neutral");
    }
  });
});
