import {
  getRiskTKey,
  parseRiskLevel,
  portfolioRiskColor,
  portfolioRiskTKey,
  StockRiskLevel,
} from "@/lib/stock-analysis-utils";
import { describe, expect, it } from "vitest";

// 后端 compute_risk_level（crates/analysis-engine/src/portfolio_monitor.rs）的产出值域，
// 逐字取自它的 return 语句。前端值域表必须覆盖它，否则掉进末尾的 MID 兜底。
describe("parseRiskLevel 覆盖后端 compute_risk_level 值域", () => {
  it("低风险 → LOW", () => {
    expect(parseRiskLevel("低风险")).toBe(StockRiskLevel.LOW);
  });

  it("中等风险 → MID", () => {
    expect(parseRiskLevel("中等风险")).toBe(StockRiskLevel.MID);
  });

  it("高风险 → HIGH", () => {
    expect(parseRiskLevel("高风险")).toBe(StockRiskLevel.HIGH);
  });

  // 本轮修复的核心：零持仓时后端返回「无持仓」，它不是风险高低的一档。
  // 修复前它不在值域表内 ⇒ 掉到 MID 兜底 ⇒ 空仓在界面上显示成「中风险」。
  it("无持仓 → UNRATED，且文案不是「中风险」", () => {
    expect(parseRiskLevel("无持仓")).toBe(StockRiskLevel.UNRATED);
    expect(getRiskTKey("无持仓")).toBe("stockAnalysis.noHoldings");
  });

  it("负控：真·中风险不得被 UNRATED 分支带走", () => {
    expect(parseRiskLevel("中风险")).toBe(StockRiskLevel.MID);
    expect(getRiskTKey("中风险")).toBe("stockAnalysis.riskMid");
  });

  it("负控：未识别的自由文本仍兜底 MID（decision-agreement 的 RISK_RANK 依赖此语义）", () => {
    expect(parseRiskLevel("完全不是风险等级")).toBe(StockRiskLevel.MID);
  });

  // 下面这组锁的是**决策链**那套值域（rhai `overall_risk`：低/中/高/极高）。
  // 集中度算法另有一套档位，见文件下半部分的 portfolioRiskTKey。
});

// 集中度算法（后端 portfolio_monitor::compute_risk_level）产出的是**另一套**档位，
// 与上面那套决策链枚举不是一回事。这里逐个锁住它自己的呈现。
describe("portfolioRiskTKey 覆盖集中度算法的档位", () => {
  it("低风险 / 中等风险 / 高风险 各有对应译文键", () => {
    expect(portfolioRiskTKey("低风险")).toBe("stockAnalysis.riskLow");
    expect(portfolioRiskTKey("中等风险")).toBe("stockAnalysis.riskMid");
    expect(portfolioRiskTKey("高风险")).toBe("stockAnalysis.riskHigh");
  });

  // 本轮修复的核心：它此前经子串扫描命中「高风险」⇒ 虚高一级上屏。
  it("中高风险 用它自己的键，不得落到 riskHigh", () => {
    expect(portfolioRiskTKey("中高风险")).toBe("stockAnalysis.riskMidHigh");
    expect(portfolioRiskTKey("中高风险")).not.toBe("stockAnalysis.riskHigh");
  });

  it("无持仓 → 暂无持仓，不是风险档", () => {
    expect(portfolioRiskTKey("无持仓")).toBe("stockAnalysis.noHoldings");
  });

  it("负控：未识别输入兜底 riskMid，不得静默当空", () => {
    expect(portfolioRiskTKey("完全不是风险等级")).toBe("stockAnalysis.riskMid");
    expect(portfolioRiskTKey(undefined)).toBe("stockAnalysis.riskMid");
  });

  it("颜色与文案同判据：中高风险与高风险同色，无持仓走中性", () => {
    expect(portfolioRiskColor("中高风险")).toBe(portfolioRiskColor("高风险"));
    expect(portfolioRiskColor("无持仓")).toBe("var(--muted)");
    expect(portfolioRiskColor("低风险")).not.toBe(portfolioRiskColor("高风险"));
  });
});
