// SPDX-License-Identifier: AGPL-3.0-only

import { actionToDirection, parseAction, StockAction } from "@/lib/stock-analysis-utils";
import type { StockDecision } from "@/types";
import { Button, Tag, Tooltip } from "antd";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

/**
 * 决策可信度受限提示 — 把「这次决策靠不靠得住」显式摆到决策卡上。
 *
 * 存在意义（解决「黑盒观望」体感）：
 * 用户在决策卡上只看到「观望 / 仓位 0%」，但同一个 `观望` 至少有三种性质完全
 * 不同的来源，此前界面上无法区分：
 *   ① 数据不足被动降级 —— 上游节点缺失 / 权重坍缩，系统不敢给方向；
 *   ② 分析后主动看空 —— 证据充分，结论就是不该买；
 *   ③ 中性无可操作信号 —— 多空相抵。
 * 三者对用户的意义天差地别（①要补数据或谨慎、②是明确结论），但 UI 上一模一样。
 * 本组件把 ① 的前提（因子权重坍缩 + 数据缺口清单）与「被动降级」判定直接摊开。
 *
 * 数据来源（均已在 `lib/agentOutput.ts::normalizeDecision` 落为 camelCase）：
 *   · `decision.weightsCollapsed` / `collapseReason` / `weightRatio` / `untrustedCount`
 *     — portfolio-mgr.rhai V66 输出，判定因子权重是否整体坍缩；
 *   · `decision.dataGaps` — portfolio-mgr 消费的上游节点缺失清单（后端原名
 *     `data_gaps`，为决策 JSON 里唯一的顶层 snake_case 字段）。
 *
 * 注意：这些字段在 2026-09-11 之前被 `normalizeDecision` 的白名单构造丢弃，
 * 导致 DecisionBanner 里早已写好的 collapse Tag 从未显示过。若本组件不生效，
 * 先查解析层而不是渲染层。
 *
 * 2026-09-21：「被动降级」判据收紧为**必须权重坍缩**（`weightsCollapsed === true`）。
 * 旧判据把「仅一项数据缺口」也算降级 —— 688114 实证：`collapseReason=none`、
 * 三路 action 全「观望」、action 维度 30/30，仍被渲染成「数据不足导致的被动降级」。
 * 仅有缺口时改用 `trustNotice.gapsNotDegraded` 文案：缺口照旧完整展示，
 * 但不再声称方向被降级（缺口的后果是「证据少」，不是「方向被压」）。
 */

interface Props {
  decision: StockDecision;
  /**
   * 展示形态：
   * - `banner`：整条警示（专业模式 / 详情区），带可展开的数据缺口清单；
   * - `tag`：单枚小标签 + Tooltip（简洁模式 / 工具栏，空间受限处）。
   */
  variant?: "banner" | "tag";
  /** 是否显示「后果」说明（权重坍缩对仓位与置信度的实际影响） */
  showConsequence?: boolean;
}

export function DecisionTrustNotice({ decision, variant = "banner", showConsequence = true }: Props) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);

  const collapsed = decision.weightsCollapsed === true;
  const gaps = decision.dataGaps ?? [];

  /** 权重坍缩原因文案（与 DecisionBanner 原有口径一致，避免两处各写一套映射） */
  const collapseText = useMemo(() => {
    if (!collapsed) { return ""; }
    switch (decision.collapseReason) {
      case "dqi_collapsed":
        return t("stockAnalysis.weightCollapseDqi");
      case "multi_untrusted":
        return t("stockAnalysis.weightCollapseUntrusted", {
          count: decision.untrustedCount ?? "?",
        });
      default:
        return t("stockAnalysis.weightCollapseThreshold", {
          ratio: decision.weightRatio ?? "?",
        });
    }
  }, [collapsed, decision.collapseReason, decision.untrustedCount, decision.weightRatio, t]);

  // 无任何可信度问题时不渲染 —— 避免每张决策卡都挂一条无信息量的提示
  if (!collapsed && gaps.length === 0) { return null; }

  // 「被动降级」判定：结论是「不操作」，且存在可信度限制。
  // 此时的不操作不代表看空，而是系统在证据不足时的保守选择 —— 必须说清楚。
  //
  // P0 修复(2026-09-12): 旧条件 `action === "WAIT" || positionPct <= 0` 把「零仓位」当成
  // 「不操作」的证据，漏掉了方向性决策天然零仓位这一事实 —— 卖出 / 减持的 positionPct
  // 恒为 0（portfolio-mgr.rhai「action 决定了仓位性质」：final_action == "卖出" → 仓位清零）。
  // 于是「主动看空」被渲染成「本次「观望」为数据不足导致的被动降级，非看空判断」，
  // 与同一张卡片上的「卖出」标签直接打架，并且把方向说反（看空 → 非看空判断）。
  // 现显式排除方向性 action：只有 HOLD / WAIT（或无方向）才可能是被动降级。
  //
  // P1-6(2026-09-14): 判定改走统一归一化。上一版修复写的是**英文字面量**
  // （`action === "BUY"` 等），而 portfolio-mgr 实际产出的是中文「买入 / 增持 /
  // 卖出 / 减持」—— 一个都匹配不上，于是中文方向性决策仍被判成「非方向性」，
  // 又落回「本次观望为数据不足导致的被动降级，非看空判断」的文案（说的是反话）。
  // 现用 `actionToDirection` 归一化判定，中英文值域都覆盖。
  const isDirectional = actionToDirection(decision.action) !== null;
  // P0 修复(2026-09-21): 「被动降级」必须由**真降级**证据支撑 —— 即因子权重坍缩。
  // 旧条件 `(collapsed || gaps.length > 0)` 让「仅一项数据缺口、权重完全没坍缩」
  // 也渲染「本次「观望」为数据不足导致的被动降级，非看空判断」，与同一张卡片上
  // 三路 action 全一致的事实直接打架（688114 实证：formulaAction / llmAction /
  // f7FreeAction 均「观望」、action 维度拿满 30/30、`collapseReason=none`、
  // 仓位 8.4% ⇒ 结论方向本不受任何降级影响）。
  // 现分流：仅缺口 ⇒ 走 `gapsNotDegraded`（缺口照旧完整展示，但不声称「降级」）。
  const isPassiveDowngrade = !isDirectional
    && (parseAction(decision.action) === StockAction.WAIT || decision.positionPct <= 0)
    && collapsed;
  /** 有数据缺口但**未**触发降级（权重未坍缩）—— 缺口的后果是「证据少」，不是「方向被压」 */
  const isGapsOnly = !collapsed && gaps.length > 0;

  const consequenceText = collapsed && showConsequence
    ? t("stockAnalysis.weightCollapseConsequence")
    : "";

  // ── 紧凑形态：单枚 Tag + Tooltip ──
  if (variant === "tag") {
    return (
      <Tooltip
        title={
          <div className="text-xs space-y-1">
            <div className="font-medium">{t("stockAnalysis.trustNotice.title")}</div>
            {isPassiveDowngrade && <div>{t("stockAnalysis.trustNotice.passiveWatch")}</div>}
            {isGapsOnly && <div>{t("stockAnalysis.trustNotice.gapsNotDegraded")}</div>}
            {collapsed && <div>{collapseText}</div>}
            {gaps.length > 0 && (
              <div>
                {t("stockAnalysis.trustNotice.gapReason", { count: gaps.length })}
                {"："}
                {gaps.join("、")}
              </div>
            )}
            {consequenceText && <div>{consequenceText}</div>}
          </div>
        }
      >
        <Tag color="orange" style={{ margin: 0, fontSize: 11, lineHeight: "18px", paddingInline: 4 }}>
          ⚠️ {t("stockAnalysis.trustNotice.tagLabel")}
        </Tag>
      </Tooltip>
    );
  }

  // ── 完整形态：警示条 + 可展开缺口清单 ──
  return (
    <div
      className="rounded px-2.5 py-1.5 text-xs"
      style={{
        background: "rgba(250, 173, 20, 0.10)",
        borderLeft: "3px solid var(--sa-amber)",
      }}
    >
      <div className="flex items-center gap-2 flex-wrap">
        <span className="font-semibold" style={{ color: "var(--sa-amber)" }}>
          ⚠️ {t("stockAnalysis.trustNotice.title")}
        </span>
        {isPassiveDowngrade && (
          <span style={{ color: "var(--color-text-primary)" }}>
            {t("stockAnalysis.trustNotice.passiveWatch")}
          </span>
        )}
        {isGapsOnly && (
          <span style={{ color: "var(--muted)" }}>
            {t("stockAnalysis.trustNotice.gapsNotDegraded")}
          </span>
        )}
        {collapsed && (
          <span style={{ color: "var(--muted)" }}>
            {t("stockAnalysis.trustNotice.collapseLabel")}
            {"："}
            {collapseText}
          </span>
        )}
        {gaps.length > 0 && (
          <span style={{ color: "var(--muted)" }}>
            {t("stockAnalysis.trustNotice.gapReason", { count: gaps.length })}
          </span>
        )}
        {gaps.length > 0 && (
          <Button
            type="link"
            size="small"
            style={{ height: "auto", padding: 0, fontSize: 12 }}
            onClick={() => setExpanded((v) => !v)}
          >
            {expanded
              ? t("stockAnalysis.trustNotice.hideGaps")
              : t("stockAnalysis.trustNotice.showGaps")}
          </Button>
        )}
      </div>

      {expanded && gaps.length > 0 && (
        <div className="mt-1.5 flex flex-wrap gap-1">
          {gaps.map((g) => (
            <Tag
              key={g}
              color="orange"
              style={{ margin: 0, fontSize: 11, lineHeight: "18px", paddingInline: 4 }}
            >
              {g}
            </Tag>
          ))}
        </div>
      )}

      {consequenceText && (
        <div className="mt-1" style={{ color: "var(--muted)" }}>
          {consequenceText}
        </div>
      )}
    </div>
  );
}
