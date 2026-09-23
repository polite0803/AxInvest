import {
  AGREEMENT_TOTAL_MAX,
  AGREEMENT_WEIGHTS,
  agreementSideFromRaw,
  computeAgreement,
  computeAgreementScore,
  scoreAgreementAction,
  scoreAgreementConfidence,
  scoreAgreementDataGaps,
  scoreAgreementEvidence,
  scoreAgreementPosition,
  scoreAgreementRisk,
} from "@/lib/decision-agreement";
import { describe, expect, it } from "vitest";

/**
 * 本模块是后端 `compute_decision_agreement`（V65 起 6 维 / 满分 100）的前端镜像，
 * 用于 store 里三条降级路径。测试重点不是「公式算得对」，而是**三类同源缺陷不回流**：
 *   ① 方向判据只认中文 ⇒ 英文 token 被判成「对立方向 0 分」；
 *   ② 「观望 vs 不确定」分支被靠前分支吞掉（不可达）；
 *   ③ confidence 阈值按 0~1 量纲写而实参是 0~100 ⇒ 该维度几乎恒 0；
 * 以及**刻度与后端一致**（6 维满分 100，而不是旧的 3 维 50/30/20）。
 */

describe("decision-agreement 各维满分与后端声明一致", () => {
  it("权重表合计恒为 100", () => {
    expect(AGREEMENT_TOTAL_MAX).toBe(100);
    expect(
      AGREEMENT_WEIGHTS.action
        + AGREEMENT_WEIGHTS.positionPct
        + AGREEMENT_WEIGHTS.confidence
        + AGREEMENT_WEIGHTS.riskLevel
        + AGREEMENT_WEIGHTS.dataGaps
        + AGREEMENT_WEIGHTS.evidence,
    ).toBe(100);
  });
});

describe("scoreAgreementAction —— 缺陷①英文值域 / 缺陷②不可达分支", () => {
  it("英文 token 与中文同向不再被判成「对立方向 0 分」", () => {
    // 旧实现：isBuy("buy") 为 false（不含「买」字），落到 else ⇒ 0
    expect(scoreAgreementAction("BUY", "增持")).toBe(20);
    expect(scoreAgreementAction("SELL", "减持")).toBe(20);
    expect(scoreAgreementAction("hold", "持有")).toBe(30);
  });

  it("dashboard 值域短语（强烈买入/强烈卖出）不再被判成对立", () => {
    // 权威表 `STOCK_ACTION_LABELS` 把 `强烈买入` 折叠为 BUY、`强烈卖出` 折叠为 SELL，
    // 因此与 `买入` / `卖出` 是**全等**关系 ⇒ 满分 30（旧实现落到 else ⇒ 0）。
    expect(scoreAgreementAction("强烈买入", "买入")).toBe(30);
    expect(scoreAgreementAction("强烈卖出", "卖出")).toBe(30);
  });

  it("观望 vs 不确定 = 6（旧实现被靠前的 5 分分支吞掉，恒不可达）", () => {
    // 旧实现：isWatch(观望) 先命中 `(isHold||isWatch) vs isUncertain = 5`
    expect(scoreAgreementAction("观望", "不确定")).toBe(6);
    expect(scoreAgreementAction("不确定", "观望")).toBe(6);
  });

  it("持有 vs 不确定 = 3；持有 vs 观望 = 10", () => {
    expect(scoreAgreementAction("持有", "不确定")).toBe(3);
    expect(scoreAgreementAction("持有", "观望")).toBe(10);
  });

  it("缺失哨兵（数据缺失）按「单侧缺失」档 15，不落入对立 0 分", () => {
    expect(scoreAgreementAction("数据缺失", "买入")).toBe(15);
  });

  it("UNCERTAIN（无法判断）是判断结论、非缺失哨兵 ⇒ 与明确方向比对落 0", () => {
    // `无法判断` 在权威表里是 UNCERTAIN（「无法判断方向」），不是 `数据缺失`（UNAVAILABLE）。
    // 后端同序（`(Some(_), Some(_)) => 0.0`）也把「不确定 vs 买入」判为对立档，
    // 此处刻意复刻后端而非「更合理地」给中性分 —— 两端口径必须一致。
    expect(scoreAgreementAction("无法判断", "买入")).toBe(0);
    expect(scoreAgreementAction("不确定", "买入")).toBe(0);
  });

  it("未识别值域 / 单侧缺失 = 15，明确对立 = 0", () => {
    expect(scoreAgreementAction("这段是自由文本", "买入")).toBe(15);
    expect(scoreAgreementAction(undefined, "买入")).toBe(15);
    expect(scoreAgreementAction("买入", "卖出")).toBe(0);
  });

  it("全等满分 30", () => {
    expect(scoreAgreementAction("增持", "增持")).toBe(30);
  });
});

describe("scoreAgreementPosition / scoreAgreementConfidence —— 缺陷③量纲", () => {
  it("positionPct 按百分点判定：≤10 → 20 / ≤20 → 10 / 其余 0", () => {
    expect(scoreAgreementPosition(50, 60)).toBe(20);
    expect(scoreAgreementPosition(50, 62)).toBe(10);
    expect(scoreAgreementPosition(50, 80)).toBe(0);
    // 单侧缺失不再给兜底分（后端 V65：避免虚高）
    expect(scoreAgreementPosition(50, null)).toBe(0);
    expect(scoreAgreementPosition(null, null)).toBe(0);
  });

  it("confidence 实参是 0~100：旧实现用 diff<=0.1/0.2/0.4 判定 ⇒ 几乎恒 0", () => {
    // 旧实现：diff = 10 → 既不满足 <=0.4 也不满足前两档 ⇒ 0 分
    expect(scoreAgreementConfidence(60, 70)).toBe(15);
    expect(scoreAgreementConfidence(60, 75)).toBe(10);
    expect(scoreAgreementConfidence(60, 95)).toBe(5);
    expect(scoreAgreementConfidence(10, 90)).toBe(0);
    expect(scoreAgreementConfidence(60, null)).toBe(0);
  });
});

describe("scoreAgreementRisk / data_gaps / evidence", () => {
  it("风险等级：精确 15 / 相邻 8 / 跨级 0", () => {
    expect(scoreAgreementRisk("高风险", "高风险")).toBe(15);
    expect(scoreAgreementRisk("高风险", "极高风险")).toBe(8);
    expect(scoreAgreementRisk("高风险", "低风险")).toBe(0);
    // 未识别 → MID（与后端默认中风险同档）
    expect(scoreAgreementRisk(undefined, "中风险")).toBe(15);
  });

  it("data_gaps：双空满分 / 单空中性 5 / 双方非空为 Jaccard×10", () => {
    expect(scoreAgreementDataGaps([], [])).toBe(10);
    expect(scoreAgreementDataGaps([], ["a"])).toBe(5);
    expect(scoreAgreementDataGaps(["a", "b"], ["a", "b"])).toBe(10);
    // 交集 1 / 并集 3 → 3.33
    expect(scoreAgreementDataGaps(["a", "b"], ["a", "c"])).toBeCloseTo(10 / 3, 5);
    // 归一化后大小写 / 空格不构成差异
    expect(scoreAgreementDataGaps(["ROE "], ["roe"])).toBe(10);
  });

  it("evidence：≥3 条满分 / 2 条 5 / 其余 0", () => {
    expect(scoreAgreementEvidence(3)).toBe(10);
    expect(scoreAgreementEvidence(5)).toBe(10);
    expect(scoreAgreementEvidence(2)).toBe(5);
    expect(scoreAgreementEvidence(1)).toBe(0);
    expect(scoreAgreementEvidence(undefined)).toBe(0);
  });
});

describe("computeAgreement —— 总分刻度", () => {
  it("六维全对偶得 100；旧 3 维刻度只能给到 65（缺 risk/gaps/evidence）", () => {
    const full = computeAgreement(
      { action: "买入", positionPct: 50, confidence: 70, riskLevel: "中风险", dataGaps: ["a"] },
      {
        action: "买入",
        positionPct: 50,
        confidence: 70,
        riskLevel: "中风险",
        dataGaps: ["a"],
        evidenceCitedCount: 4,
      },
    );
    expect(full?.total).toBe(100);

    // 旧记录常缺 risk/gaps/evidence ⇒ 按后端口径分别得 15 / 10 / 0
    const legacy = computeAgreement(
      { action: "买入", positionPct: 50, confidence: 70 },
      { action: "买入", positionPct: 50, confidence: 70 },
    );
    expect(legacy?.total).toBe(30 + 20 + 15 + 15 + 10 + 0);
    // 旧实现同输入给 50 + 30 + 20 = 100 ⇒ 两把尺子
    expect(legacy?.total).not.toBe(100);
  });

  it("任一侧缺失返回 null（不伪造分数）", () => {
    expect(computeAgreement(null, { action: "买入" })).toBeNull();
    expect(computeAgreement({ action: "买入" }, null)).toBeNull();
  });
});

describe("computeAgreementScore —— 降级路径主入口", () => {
  it("LLM JSON 整段不可解析 → null（旧实现会按全字段缺失硬算一个分）", () => {
    expect(computeAgreementScore({ action: "买入" }, "这不是 JSON")).toBeNull();
    expect(computeAgreementScore({ action: "买入" }, null)).toBeNull();
    expect(computeAgreementScore({ action: "买入" }, "")).toBeNull();
  });

  it("公式侧非对象 → null", () => {
    expect(computeAgreementScore(null, '{"action":"买入"}')).toBeNull();
    expect(computeAgreementScore("字符串", '{"action":"买入"}')).toBeNull();
  });

  it("能穿透 AgentNode 包装与 markdown 围栏取值", () => {
    const wrapped = JSON.stringify({
      role: "trader",
      content: "```json\n" + JSON.stringify({ action: "增持", positionPct: 50, confidence: 70 }) + "\n```",
    });
    // 公式侧 action=增持 同档（20） + pos 10 分差 20 + conf 0 分差 15
    // + risk 两侧缺失都按 MID → 15 + gaps 双空 10 + evidence 缺 0
    expect(computeAgreementScore({ action: "买入", positionPct: 50, confidence: 70 }, wrapped)).toBe(80);
  });

  it("LLM 侧 action 缺失时回退读 stance", () => {
    const raw = JSON.stringify({ stance: "增持", positionPct: 50, confidence: 70 });
    expect(computeAgreementScore({ action: "买入", positionPct: 50, confidence: 70 }, raw)).toBe(80);
  });
});

describe("agreementSideFromRaw —— 字段命名双写容错", () => {
  it("同时接受 snake_case 与 camelCase", () => {
    const side = agreementSideFromRaw({
      action: "买入",
      position_pct: 50,
      risk_level: "高风险",
      dataGaps: ["a"],
      evidenceCited: [1, 2, 3],
    });
    expect(side?.positionPct).toBe(50);
    expect(side?.riskLevel).toBe("高风险");
    expect(side?.dataGaps).toEqual(["a"]);
    expect(side?.evidenceCitedCount).toEqual([1, 2, 3]);
  });

  it("非对象（null / 字符串 / 数组）返回 null", () => {
    expect(agreementSideFromRaw(null)).toBeNull();
    expect(agreementSideFromRaw("x")).toBeNull();
    expect(agreementSideFromRaw([1])).toBeNull();
  });
});
