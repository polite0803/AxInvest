// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import {
  cleanToolCallTags,
  extractDecision,
  extractLlmField,
  extractValuationApplicability,
  normalizeDecision,
  parseDecisionExplanation,
  parseJsonLoose,
  tryParseDecision,
} from "@/lib/agentOutput";
import type { StockDecision } from "@/types/stock-analysis";

/**
 * 2026-09-21 回归：`parseJsonLoose` 曾**无条件先剥 fence**，于是当 fence 出现在某个
 * JSON 字符串值**内部**时（典型：AgentNode 包装
 * `{"role":"trader","content":"```json\n{...}\n```"}`），正则会劫持整段文本并把 src
 * 换成仍带转义的内层片段 ⇒ 对**完全合法的 JSON** 返回 null。
 */
describe("parseJsonLoose —— 内嵌 fence 不得劫持合法 JSON", () => {
  const wrapped = JSON.stringify({
    role: "trader",
    content: "```json\n" + JSON.stringify({ action: "增持" }) + "\n```",
  });

  it("外层对象合法时返回外层（fence 在 content 字符串内部）", () => {
    const parsed = parseJsonLoose(wrapped);
    expect(parsed).not.toBeNull();
    expect(parsed?.role).toBe("trader");
  });

  it("同一输入下 extractLlmField 仍能取到内层字段", () => {
    expect(extractLlmField(wrapped, "action")).toBe("增持");
  });

  it("整段被 fence 包裹（正常形态）仍能解析", () => {
    expect(parseJsonLoose('```json\n{"action":"买入"}\n```')?.action).toBe("买入");
  });

  it("前后带杂文时按首尾花括号切片兜底", () => {
    expect(parseJsonLoose('杂文 {"action":"买入"} 结尾')?.action).toBe("买入");
  });

  it("确实非法时返回 null", () => {
    expect(parseJsonLoose("这不是 JSON")).toBeNull();
    expect(parseJsonLoose("")).toBeNull();
    expect(parseJsonLoose(null)).toBeNull();
  });

  it("数组不算对象（返回 null）", () => {
    expect(parseJsonLoose("[1,2]")).toBeNull();
  });
});

describe("agentOutput decision parsing", () => {
  const expectedDecision: StockDecision = {
    action: "BUY",
    // normalizeDecision 恒定输出该键（缺失输入时为 null）。此前漏更新本夹具，
    // 导致该文件 3 个用例长期红灯（掩盖后续新增失败）。
    // 注：2026-09-22 起 positionState 仅供展示（建议持仓状态），不参与展示档判定。
    positionState: null,
    positionPct: 20,
    targetPrice: null,
    stopLoss: null,
    horizonPriceMap: null,
    decisionsByHorizon: null,
    reasoning: "Test decision",
    riskLevel: "MID",
    confidence: 85,
    decisionConfidence: null,
    signalStrength: null,
    timeHorizon: null,
    expectedHoldingDays: null,
    targetTimeframe: null,
    adjustedConfidence: undefined,
    agreementBreakdown: undefined,
    // 决策前提字段（2026-09-11 新增透传）：缺省输入下收敛为安全默认，
    // 使「未提供」与「明确为 false」在使用处可区分（见文末防回归 describe）。
    weightsCollapsed: false,
    collapseReason: undefined,
    weightRatio: undefined,
    untrustedCount: undefined,
    dataGaps: undefined,
    isContradictory: false,
    crossCheck: undefined,
  };

  it("parses plain JSON string into StockDecision", () => {
    const parsed = tryParseDecision('{"action":"BUY","positionPct":20,"confidence":85,"reasoning":"Test decision"}');
    expect(parsed).toEqual(expectedDecision);
  });

  it("parses JSON decision wrapped in markdown code block", () => {
    const parsed = tryParseDecision(
      `Here is the decision:\n\n\`\`\`json\n{\n  "action": "BUY",\n  "positionPct": 20,\n  "confidence": 85,\n  "reasoning": "Test decision"\n}\n\`\`\``,
    );
    expect(parsed).toEqual(expectedDecision);
  });

  it("parses escaped JSON string output", () => {
    const escaped = '"{"action":"BUY","positionPct":20,"confidence":85,"reasoning":"Test decision"}"';
    const parsed = tryParseDecision(escaped);
    expect(parsed).toEqual(expectedDecision);
  });

  it("returns null for JSON arrays in raw string output", () => {
    const parsed = tryParseDecision('[{"action":"BUY","positionPct":20,"confidence":85}]');
    expect(parsed).toBeNull();
  });

  it("extractDecision returns null for array object values", () => {
    const parsed = extractDecision([{ action: "BUY" }] as unknown);
    expect(parsed).toBeNull();
  });
});

describe("normalizeDecision - 全零空壳检测", () => {
  it("空对象 {} → null (没有可解析字段)", () => {
    expect(normalizeDecision({})).toBeNull();
  });

  it("空字段对象 { action: null } → null", () => {
    expect(normalizeDecision({ action: null })).toBeNull();
  });

  it("空字符串字段对象 → null", () => {
    expect(normalizeDecision({ action: "", confidence: "", reasoning: "  " })).toBeNull();
  });

  it("HOLD 是合法决策（即便置信度为 0）→ 保留", () => {
    const parsed = normalizeDecision({ action: "HOLD" });
    expect(parsed).not.toBeNull();
    expect(parsed?.action).toBe("HOLD");
  });

  it("只含 reasoning 字段 → 保留", () => {
    const parsed = normalizeDecision({ reasoning: "基本面恶化，建议观望" });
    expect(parsed).not.toBeNull();
    expect(parsed?.reasoning).toBe("基本面恶化，建议观望");
  });

  it("snake_case 全零空壳 { position_pct: 0 } → null", () => {
    expect(normalizeDecision({ position_pct: 0 })).toBeNull();
  });

  it("CodeNode 包装但 params 是空对象 → null", () => {
    expect(normalizeDecision({ status: "ok", params: {} })).toBeNull();
  });

  it("CodeNode 包装 + 内部含有效 action → 保留", () => {
    const parsed = normalizeDecision({ status: "ok", params: { action: "BUY", confidence: 80 } });
    expect(parsed?.action).toBe("BUY");
    expect(parsed?.confidence).toBe(80);
  });
});

describe("cleanToolCallTags", () => {
  it("removes generic Hermes/Qwen-style <tool_call> blocks with <function> and <parameter>", () => {
    const input =
      "<tool_call> <function=search_stock> <parameter=stock_code> 301302 </parameter> </function> </tool_call>";
    expect(cleanToolCallTags(input)).toBe("");
  });

  it("preserves surrounding text when tool_call is embedded", () => {
    const input =
      "分析完成。<tool_call> <function=search_stock> <parameter=stock_code> 301302 </parameter> </function> </tool_call>该股票基本面良好。";
    const cleaned = cleanToolCallTags(input);
    expect(cleaned).toBe("分析完成。该股票基本面良好。");
  });

  it("removes multiple tool_call blocks", () => {
    const input =
      "<tool_call><function=foo><parameter=x>1</parameter></function></tool_call>中间文本<tool_call><function=bar><parameter=y>2</parameter></function></tool_call>";
    const cleaned = cleanToolCallTags(input);
    expect(cleaned).toBe("中间文本");
  });

  it("removes orphan <function> and <parameter> tags without outer tool_call", () => {
    const input = "<function=search_stock><parameter=stock_code>301302</parameter></function>";
    expect(cleanToolCallTags(input)).toBe("");
  });

  it("removes tool_call with attributes", () => {
    const input =
      '<tool_call id="tc1"><function=search_stock><parameter=stock_code>301302</parameter></function></tool_call>';
    expect(cleanToolCallTags(input)).toBe("");
  });

  it("still handles provider-prefixed XML tool_call format", () => {
    const input = '<anthropic:tool_call>{"name":"search_stock"}</anthropic:tool_call>实际分析内容';
    expect(cleanToolCallTags(input)).toBe("实际分析内容");
  });

  it("does not match HTML-like tags such as <figure> or <param>", () => {
    const input = '<figure>图表说明</figure><param name="x" value="1">';
    expect(cleanToolCallTags(input)).toBe(input);
  });
});

describe("normalizeDecision - workflow results map 兜底（修复'决策信息缺失'误报）", () => {
  it("识别 workflow results map 并从 portfolio-mgr.result 提取决策", () => {
    // 模拟后端 stock-analysis 工作流 output_schema 未用 $source 标记,
    // filter_by_schema fallback 到整个 results map 写入 decisionJson
    // 的老数据格式(修复前的 bug 表现)。
    const resultsMap = {
      trigger: { status: "executed", node_id: "trigger" },
      "t-quote": { status: "ok", result: { price: 12.5 } },
      research: { status: "ok", result: { risk: "中" } },
      "portfolio-mgr": {
        status: "executed",
        language: "rhai",
        result: {
          action: "买入",
          positionPct: 50,
          confidence: 75,
          riskLevel: "中",
          reasoning: "技术面强势",
          timeHorizon: "mid",
          expectedHoldingDays: 28,
          targetTimeframe: "1m",
        },
        input_params: { totalScore: 70 },
        node_id: "portfolio-mgr",
        params: { action: "买入" },
      },
      "end-output": { status: "ok" },
    };
    const parsed = normalizeDecision(resultsMap);
    expect(parsed).not.toBeNull();
    expect(parsed?.action).toBe("BUY"); // 买入 → BUY
    expect(parsed?.positionPct).toBe(50);
    expect(parsed?.confidence).toBe(75);
    expect(parsed?.riskLevel).toBe("MID"); // 中 → MID
    expect(parsed?.reasoning).toBe("技术面强势");
    expect(parsed?.timeHorizon).toBe("mid");
    expect(parsed?.expectedHoldingDays).toBe(28);
  });

  it("portfolio-mgr 是 CodeNode 包装但 .result 缺失时降级用 portfolio-mgr 本身", () => {
    // 模拟异常路径:portfolio-mgr 包装存在但 .result 字段缺失
    const resultsMap = {
      "portfolio-mgr": {
        status: "executed",
        language: "rhai",
        // result 字段缺失
        input_params: {},
        node_id: "portfolio-mgr",
        params: { action: "HOLD", confidence: 30, riskLevel: "HIGH" },
      },
    };
    const parsed = normalizeDecision(resultsMap);
    // 兜底逻辑会从 portfolio-mgr 本身提取,原 CodeNode 检测会从 .params 拿
    expect(parsed).not.toBeNull();
    expect(parsed?.action).toBe("HOLD");
    expect(parsed?.confidence).toBe(30);
    expect(parsed?.riskLevel).toBe("HIGH");
  });

  it("results map 内 portfolio-mgr 也不存在时仍返回 null（避免误报）", () => {
    // 类似 results map 结构但缺 portfolio-mgr 节点（异常工作流）
    const resultsMap = {
      trigger: { status: "ok" },
      research: { status: "ok" },
    };
    const parsed = normalizeDecision(resultsMap);
    // 没有 portfolio-mgr 节点,无法提取决策 → 保持 null
    expect(parsed).toBeNull();
  });

  it("业务决策对象含 action 字段时不被识别为 results map（不递归）", () => {
    // 即使业务决策对象恰好有一个键叫 "research"（罕见但可能）,
    // 因为它已经有 action 字段,不应被误判为 results map。
    const businessDecision = {
      action: "BUY",
      confidence: 80,
      research: "n/a", // 字符串而非对象,也不会触发检测
    };
    const parsed = normalizeDecision(businessDecision);
    expect(parsed?.action).toBe("BUY");
    expect(parsed?.confidence).toBe(80);
  });
});

// i18n-exempt: 测试内断言值，模拟后端 Rhai 输出的中文业务字段，非 UI 文案。
describe("normalizeDecision 决策前提字段透传（防回归）", () => {
  // 背景（2026-09-11 修复）：该函数此前用**白名单构造** return 对象，
  // `weightsCollapsed` / `collapseReason` / `weightRatio` / `untrustedCount` /
  // `data_gaps` / `isContradictory` / `crossCheck` 全部被静默丢弃
  // —— 于是 DecisionBanner 里早已写好的「因子权重坍缩」Tag 与跨系统互证 UI
  // 从未显示过，用户在决策卡上只看到「观望 / 0%」，无法判断这是
  // 「数据不足被动降级」还是「分析后主动看空」。这组测试锁死解析层不得丢字段。

  it("保留 V66 因子权重坍缩字段（camelCase 输入）", () => {
    const d = normalizeDecision({
      action: "观望",
      confidence: 50,
      weightsCollapsed: true,
      collapseReason: "dqi_collapsed",
      weightRatio: 12.5,
      untrustedCount: 2,
    });
    expect(d?.weightsCollapsed).toBe(true);
    expect(d?.collapseReason).toBe("dqi_collapsed");
    expect(d?.weightRatio).toBe(12.5);
    expect(d?.untrustedCount).toBe(2);
  });

  it("保留 snake_case 形式的坍缩字段（兼容变体输入）", () => {
    const d = normalizeDecision({
      action: "观望",
      confidence: 50,
      weights_collapsed: true,
      collapse_reason: "low_weight_ratio",
      weight_ratio: 8,
      untrusted_count: 3,
    });
    expect(d?.weightsCollapsed).toBe(true);
    expect(d?.collapseReason).toBe("low_weight_ratio");
    expect(d?.weightRatio).toBe(8);
    expect(d?.untrustedCount).toBe(3);
  });

  it("保留 data_gaps（后端 portfolio-mgr 顶层 snake_case 字段名）", () => {
    const gaps = ["资金流向(t-hotmoney-data)", "公告数据(t-catalyst-data)"];
    const d = normalizeDecision({ action: "观望", confidence: 45, data_gaps: gaps });
    expect(d?.dataGaps).toEqual(gaps);
  });

  it("data_gaps 为空数组或非字符串项时收敛（避免渲染空提示 / 脏数据）", () => {
    expect(normalizeDecision({ action: "观望", confidence: 45, data_gaps: [] })?.dataGaps)
      .toBeUndefined();
    expect(
      normalizeDecision({ action: "观望", confidence: 45, data_gaps: ["a", 1, null] })?.dataGaps,
    ).toEqual(["a"]);
  });

  it("保留 crossCheck 跨系统互证字段（hooks.rs 注入）", () => {
    const crossCheck = { recoConfidence: 70, divergent: true };
    const d = normalizeDecision({ action: "观望", confidence: 45, crossCheck });
    expect(d?.crossCheck).toEqual(crossCheck);
  });

  it("保留 isContradictory 自相矛盾标记", () => {
    const d = normalizeDecision({ action: "持有", confidence: 60, isContradictory: true });
    expect(d?.isContradictory).toBe(true);
  });

  it("未提供这些字段时收敛为安全默认（不误报「可信度受限」）", () => {
    const d = normalizeDecision({ action: "买入", confidence: 80, positionPct: 20 });
    expect(d?.weightsCollapsed).toBe(false);
    expect(d?.collapseReason).toBeUndefined();
    expect(d?.dataGaps).toBeUndefined();
    expect(d?.crossCheck).toBeUndefined();
    expect(d?.isContradictory).toBe(false);
  });
});

/**
 * decision-explainer 节点输出的解析（2026-09-14 补的「接出口」配套单测）。
 *
 * 该节点的产出此前零消费端（只写进 blackboard_snapshot 无人读）。接入前端后，
 * 两种历史存储形态都必须能解出来：
 *   · 实时路径 `results["decision-explainer"]` —— AgentNode 包装，业务 JSON 在 content 内层；
 *   · 回放路径 `blackboard_snapshot["decision-explainer"]` —— 旧记录经
 *     `extract_node_text` 压平为字符串，新记录（is_structured 白名单）为包装对象。
 */
describe("parseDecisionExplanation", () => {
  const payload = {
    summary: "最终裁决：观望，仓位 0%，置信度 58.7",
    explanation: "风控门 R-206 将凯利建议仓位下调至 0%",
    rule_trace: [
      { rule_id: "R-206", status: "DOWNGRADED", description: "单股仓位超上限已下调" },
      { rule_id: "R-200", status: "VETOED", description: "极高风险档位禁止持仓" },
    ],
    risk_comment: "行业分类缺失，风险等级可能被低估",
    confidence_note: "置信度受数据完整度限制",
  };

  it("解 AgentNode 包装形态（content 内层 JSON 字符串）", () => {
    const wrapper = { role: "explainer", content: JSON.stringify(payload), node_id: "decision-explainer" };
    const r = parseDecisionExplanation(wrapper);
    expect(r?.summary).toBe(payload.summary);
    expect(r?.explanation).toBe(payload.explanation);
    expect(r?.riskComment).toBe(payload.risk_comment);
    expect(r?.confidenceNote).toBe(payload.confidence_note);
    expect(r?.ruleTrace).toEqual([
      { ruleId: "R-206", status: "DOWNGRADED", description: "单股仓位超上限已下调" },
      { ruleId: "R-200", status: "VETOED", description: "极高风险档位禁止持仓" },
    ]);
  });

  it("解纯 JSON 字符串形态（旧版 snapshot 压平后的值）", () => {
    const r = parseDecisionExplanation(JSON.stringify(payload));
    expect(r?.summary).toBe(payload.summary);
    expect(r?.ruleTrace).toHaveLength(2);
  });

  it("解裸对象形态", () => {
    expect(parseDecisionExplanation(payload)?.ruleTrace[0].ruleId).toBe("R-206");
  });

  it("兼容 camelCase 字段名（prompt 调整后的形态）", () => {
    const r = parseDecisionExplanation({
      summary: "s",
      ruleTrace: [{ ruleId: "R-401", status: "VETOED", description: "d" }],
      riskComment: "rc",
      confidenceNote: "cn",
    });
    expect(r?.ruleTrace[0].ruleId).toBe("R-401");
    expect(r?.riskComment).toBe("rc");
  });

  it("跳过缺 rule_id 的规则项（不臆造编号）", () => {
    const r = parseDecisionExplanation({
      summary: "s",
      rule_trace: [
        { status: "PASS", description: "无编号" },
        { rule_id: "R-207", status: "PASS", description: "有效" },
        { rule_id: "", status: "PASS", description: "空编号" },
        "not-an-object",
      ],
    });
    expect(r?.ruleTrace).toEqual([{ ruleId: "R-207", status: "PASS", description: "有效" }]);
  });

  it("四个内容字段全空 ⇒ 返回 null（不渲染空壳面板）", () => {
    expect(parseDecisionExplanation({})).toBeNull();
    expect(parseDecisionExplanation({ summary: "  ", rule_trace: [] })).toBeNull();
    expect(parseDecisionExplanation(null)).toBeNull();
    expect(parseDecisionExplanation(undefined)).toBeNull();
    expect(parseDecisionExplanation("not json")).toBeNull();
  });

  it("只有 rule_trace 也算有效产出（不因缺摘要而丢弃）", () => {
    const r = parseDecisionExplanation({
      rule_trace: [{ rule_id: "R-208", status: "VETOED", description: "风控否决" }],
    });
    expect(r).not.toBeNull();
    expect(r?.summary).toBeNull();
    expect(r?.ruleTrace).toHaveLength(1);
  });
});

/**
 * 2026-09-21：估值适用性提取（`portfolio-mgr` 产物 → 估值面板标注）。
 *
 * 该字段此前**只写在产物里、前端零消费** ⇒ 用户看到「内在价值 80.63–156.77 元」
 * 却看不到「该区间锚定于近 5 年正净利均值 ×0.90 的历史代理」。
 * 下面同时覆盖「提得到」和「提不到时必须返回 null 而不是伪造默认值」。
 */
describe("extractValuationApplicability", () => {
  /** 取自 300308 实际产物形状（`applicable` 在 F1 生效后为 false）。 */
  const payload = {
    action: "增持",
    valuationApplicability: {
      dcfApplicable: false,
      dcfLegUsed: false,
      grahamLegUsed: true,
      reason: "净利为正但当期真实自由现金流与盈利量级脱钩（FCF/净利 = 0.14 < 0.3）",
      anchorIsFallback: true,
      grahamGrowthClamped: true,
    },
  };

  it("纯 JSON 字符串 ⇒ 直接提得到", () => {
    const a = extractValuationApplicability(JSON.stringify(payload));
    expect(a).not.toBeNull();
    expect(a?.dcfApplicable).toBe(false);
    expect(a?.dcfLegUsed).toBe(false);
    expect(a?.grahamLegUsed).toBe(true);
    expect(a?.anchorIsFallback).toBe(true);
    expect(a?.grahamGrowthClamped).toBe(true);
    expect(a?.reason).toContain("0.14");
  });

  it('AgentNode 包装 {content: "<json>"} ⇒ 提得到', () => {
    const a = extractValuationApplicability({ role: "portfolio-manager", content: JSON.stringify(payload) });
    expect(a?.dcfApplicable).toBe(false);
    expect(a?.grahamGrowthClamped).toBe(true);
  });

  it("CodeNode 包装 {result: {...}} ⇒ 提得到", () => {
    const a = extractValuationApplicability({ status: "ok", result: payload });
    expect(a?.dcfApplicable).toBe(false);
  });

  it("旧模板（没有该字段）⇒ 返回 null，**不伪造**默认块", () => {
    // 关键：「产物里没这个字段」与「字段说前提成立」是两件事。
    // 若此处返回一个 {dcfApplicable:true,...} 的默认值，面板就会显示一条假的「适用」。
    expect(extractValuationApplicability(JSON.stringify({ action: "买入", positionPct: 20 }))).toBeNull();
    expect(extractValuationApplicability({ action: "买入" })).toBeNull();
    expect(extractValuationApplicability(null)).toBeNull();
    expect(extractValuationApplicability(undefined)).toBeNull();
    expect(extractValuationApplicability("")).toBeNull();
    expect(extractValuationApplicability("not json")).toBeNull();
  });

  it("字段缺省时按「不误报」取向：dcfApplicable 缺省 true，其余缺省 false", () => {
    const a = extractValuationApplicability({ valuationApplicability: {} });
    expect(a).not.toBeNull();
    // dcfApplicable 缺省 true：旧模板未注入 `applicable` 时行为应与改动前一致
    expect(a?.dcfApplicable).toBe(true);
    // 其余缺省 false：不凭空制造「锚定是代理 / 增长率被封顶」的警报
    expect(a?.anchorIsFallback).toBe(false);
    expect(a?.grahamGrowthClamped).toBe(false);
    expect(a?.dcfLegUsed).toBe(false);
    expect(a?.grahamLegUsed).toBe(false);
    expect(a?.reason).toBe("");
  });

  it("缺省值不是恒真/恒假：显式给值必须被采纳（区分力对照）", () => {
    const off = extractValuationApplicability({
      valuationApplicability: { dcfApplicable: true, anchorIsFallback: false, grahamGrowthClamped: false },
    });
    const on = extractValuationApplicability({
      valuationApplicability: { dcfApplicable: false, anchorIsFallback: true, grahamGrowthClamped: true },
    });
    // 两个方向都覆盖 ⇒ 证明上一用例的缺省断言不是「怎么给都这样」
    expect(off?.dcfApplicable).toBe(true);
    expect(on?.dcfApplicable).toBe(false);
    expect(off?.anchorIsFallback).toBe(false);
    expect(on?.anchorIsFallback).toBe(true);
    expect(off?.grahamGrowthClamped).toBe(false);
    expect(on?.grahamGrowthClamped).toBe(true);
  });

  it("非布尔/非字符串的脏值不得被当成 true（避免假标注）", () => {
    const a = extractValuationApplicability({
      valuationApplicability: {
        dcfApplicable: "false", // 字符串 "false" 是**真值**，直接当布尔会把不适用读成适用
        anchorIsFallback: 1,
        grahamGrowthClamped: "yes",
        reason: 42,
      },
    });
    expect(a?.dcfApplicable).toBe(true); // 非 `false` 字面量 ⇒ 保持缺省 true（不误报不适用）
    expect(a?.anchorIsFallback).toBe(false);
    expect(a?.grahamGrowthClamped).toBe(false);
    expect(a?.reason).toBe("");
  });
});
