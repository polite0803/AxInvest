import { useTranslation } from "react-i18next";

/** 需要声明「本环节不按档」的面板组（三句各自点名自己的主语，不共用一条占位串）。 */
export type HorizonScope = "analyst" | "debate" | "risk";

const SCOPE_KEY: Record<HorizonScope, string> = {
  analyst: "stockAnalysis.horizonScopeAnalyst",
  debate: "stockAnalysis.horizonScopeDebate",
  risk: "stockAnalysis.horizonScopeRisk",
};

/**
 * 「本环节哪些轴按档、哪些轴仍是共用一份」的显式声明（PLAN §五十三 ⑤ 甲落地，
 * §五十四 B1(v128) 后 risk 一格改写，#45(v129) 再改写一次）。
 *
 * 为什么必须有：R-11 原设计是四路分支各挂该档分析师子集 / 对抗 / 风险，现状落了
 * 「评分 + 决策 + 出场」三格；v128 给「风险」加了一格（**只加一根轴**），v129 把那一格
 * 接上了整条主链：
 *   · 按档：本档持仓窗口（2/5/28/90 交易日）的回撤深度（`cls-risk-level-{tier}` 四节点），
 *     以及**消费侧**的 f4 风险证据、`risk_bias`、仓位上限与 `pm_risk_veto`
 *     —— 自 v129 起它们读的都是「按所选档收紧后的 `overall_risk`」（两条轴都只升不降）
 *   · 按档：**分析师**（v133 B2-2）——实例由 `Period::analyst_subset()` 逐档展开，
 *     每档各跑自己的子集并按该档契约各自产出 VERDICT。本格文案此前长期仍写「四档共用同一批
 *     分析师输出」，而同一屏下的 `AnalystReportGrid` 早已按档列实例（见其 `tierOrderOf`）——
 *     正是本文件下方「声明落后于数据层」那条禁物。2026-10-16 收口（`PLAN-llm-horizon-decision-and-parity.md` §6.3）。
 *   · 按档：**辩论**（v161 A4 起每档各跑一组 `bull-rN--<档>` / `bear-rN--<档>`，四份记录互不覆盖）
 *   · 仍共用一份：波动率 / 夏普（60 日全局）、基本面四项（最新财报）
 *
 * ⇒ 三种伪装都不许：把共用内容复制四遍（读者会以为是四份独立判断）、
 *   什么都不说（同屏「四档并列」+「一份输入」而无说明会被读成缺陷），
 *   以及**声明落后于数据层**（v128 那句「f4 与 risk_bias 仍用全局档」在 v129 之后就是假话 ——
 *   写窄了同样是伪装）。声明必须与 `risk-level.rhai` 的 `axisScope` 同判据，
 *   这一条由 `HorizonScopeNotice.test.tsx` 的判据词表 + 三条负控锁定。
 *
 * 判据词表的正主是 `risk-level.rhai` 输出的 `axisScope` 字段（Rust 侧由
 * `seeded_template_carries_horizon_scoped_risk_nodes` 锁），改判据必须三处同批。
 *
 * 分析师一格已随 B2-2 收口，辩论一格已随 A4（v161 逐档辩论）收口 —— 两格的文案都与数据层
 * 同批改口，判据形态见 `HorizonScopeNotice.test.tsx`（旧句一律降级成负控样本）。
 */
export function HorizonScopeNotice({ scope }: { scope: HorizonScope }) {
  const { t } = useTranslation();
  return (
    <div
      data-testid="horizon-scope-notice"
      className="text-xs"
      style={{ color: "var(--muted)", marginBottom: 4 }}
    >
      {t(SCOPE_KEY[scope])}
    </div>
  );
}
