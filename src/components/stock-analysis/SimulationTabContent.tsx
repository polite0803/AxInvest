import { translateBackendError } from "@/lib/errorI18n";
import { useStockAnalysisStore } from "@/stores";
import type { SimulationSnapshot } from "@/stores";
import { Tabs, Tag } from "antd";
import { Info, Zap } from "lucide-react";
import { useTranslation } from "react-i18next";
import { MarketSimPanel } from "./MarketSimPanel";
import { MonteCarloPanel } from "./MonteCarloPanel";
import { QuantSimPanel } from "./QuantSimPanel";

/**
 * SimulationTabContent — 股票分析页的「模拟仿真」标签内容。
 *
 * 两个来源，职责分明：
 * 1. **工作流自动仿真**（顶部区块）—— 分析完成后由**后端挂钩**在决策落库之后自动
 *    跑、写入快照。模板里的 `sim-verify` 只是**图示节点**（`enabled=false` 且无边，
 *    见 `seed_stock_analysis.rs`），真正执行在 `stock_workflow/sim_hook.rs`：
 *    它在所有会改决策的节点之后触发，只写补充信息、不产出决策字段，
 *    因此不阻滞也不改写决策，也不占工作流执行时长。
 *    此处只读展示，用户无需手点（结果经 `simulation-ready` 事件回填）。
 * 2. **手动细粒度模拟**（下方二级标签）—— 三个面板按需调参重跑，用于深入探索。
 *
 * ⚠️ 读结果前必读：
 * - `survivalRate` 是**上涨场景占比**（由勾选的场景集合决定），**不代表个股质地**；
 * - `consistencyScore === null` 表示**不可判定**（各场景涨跌幅均值趋零 ⇒ 变异系数
 *   数学上无定义）。这是最分歧的情形，**不是**最一致 —— 故不可以用 0 兜底；
 * - 当前仿真市场不含「你自己的订单」（内核无 order 注入入口，见
 *   `PLAN-market-sim-value.md`），故结果不得解读为本笔交易的冲击成本；
 * - 失败原因由后端**错误码**表达（`STOCK_SIM_*`，见 `error_code.rs::stock_sim`），
 *   经 `translateBackendError` 走 11 语言顶层 `error` 段翻译。**本文件不得出现
 *   面向用户的硬编码文案，也不得直接渲染后端字段** —— 后端 `detail` 是技术串，
 *   直接渲染会让非中文界面漏出中文。
 */
export function SimulationTabContent() {
  const { t } = useTranslation();
  const stockCode = useStockAnalysisStore((s) => s.stockCode);
  const stockName = useStockAnalysisStore((s) => s.stockName);
  const quotePrice = useStockAnalysisStore((s) => s.quote?.price ?? null);
  const simulation = useStockAnalysisStore((s) => s.simulation);

  const hasTarget = Boolean(stockCode);

  /** 传给三个面板的上下文（面板内部各自决定是否需要 / 是否允许覆盖） */
  const contextProps = { stockCode, referencePriceYuan: quotePrice };

  return (
    <div className="space-y-4">
      {/* 当前标的上下文条 */}
      <div
        className="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-lg border px-3 py-2 text-sm"
        style={{ borderColor: "var(--border)", background: "var(--surface)" }}
      >
        <span className="text-secondary">{t("stockAnalysis.simulation.contextLabel")}</span>
        {hasTarget
          ? (
            <span className="font-medium">
              {stockName ? `${stockName}（${stockCode}）` : stockCode}
              {quotePrice != null && (
                <span className="ml-2 font-mono text-secondary">
                  {t("stockAnalysis.simulation.yuanEquivalent", { yuan: quotePrice.toFixed(2) })}
                </span>
              )}
            </span>
          )
          : <span className="text-secondary">{t("stockAnalysis.simulation.noStock")}</span>}
        <span className="text-xs text-secondary">· {t("stockAnalysis.simulation.contextHint")}</span>
      </div>

      {/* 用途边界说明 */}
      <div
        className="flex items-start gap-2 rounded-lg border px-3 py-2 text-xs leading-relaxed"
        style={{ borderColor: "var(--border)", background: "var(--surface)", color: "var(--color-text-secondary)" }}
      >
        <Info size={14} className="mt-0.5 shrink-0" />
        <span>{t("stockAnalysis.simulation.scope")}</span>
      </div>

      {/* 工作流自动仿真结果（决策之后自动生成，无需手点） */}
      <AutoSimulationCard simulation={simulation} />

      <Tabs
        size="small"
        items={[
          {
            key: "market",
            label: `🏭 ${t("stockAnalysis.backtest.tabSimulation")}`,
            children: <MarketSimPanel {...contextProps} />,
          },
          {
            key: "monte-carlo",
            label: `🎲 ${t("stockAnalysis.backtest.tabMonteCarlo")}`,
            children: <MonteCarloPanel {...contextProps} />,
          },
          {
            key: "quant",
            label: `🤖 ${t("stockAnalysis.backtest.tabQuantSim")}`,
            children: <QuantSimPanel {...contextProps} />,
          },
        ]}
      />
    </div>
  );
}

/**
 * AutoSimulationCard — 展示工作流 `sim-verify` 节点自动产出的仿真结果。
 *
 * 无数据时给出明确预期（「分析完成后自动生成」），而不是让用户面对一片空白
 * 去猜是否需要手点运行。
 */
function AutoSimulationCard({ simulation }: { simulation: SimulationSnapshot | null }) {
  const { t } = useTranslation();

  // 一致性档位口径与 MonteCarloPanel **完全一致**（<0.5 一致 / <1.0 可接受 / >=1.0 环境依赖）。
  // 两处若各写一套阈值，同一份数据会显示成两个结论。
  const consistency = simulation?.consistencyScore ?? null;

  return (
    <div
      className="rounded-lg border px-3 py-3"
      style={{ borderColor: "var(--border)", background: "var(--surface)" }}
    >
      <div className="flex flex-wrap items-center gap-2">
        <Zap size={14} />
        <span className="text-sm font-medium">{t("stockAnalysis.simulation.autoTitle")}</span>
        {simulation && (
          <Tag color={simulation.simOk ? "green" : "default"}>
            {simulation.simOk
              ? t("stockAnalysis.simulation.autoReady")
              : t("stockAnalysis.simulation.autoFailed")}
          </Tag>
        )}
      </div>

      {/* 尚未生成：说明何时会自动出现，避免用户以为要手点 */}
      {!simulation && <p className="mt-2 text-xs text-secondary">{t("stockAnalysis.simulation.autoPending")}</p>}

      {
        /* 已生成但失败：给出具体原因，不用笼统"失败"糊过去。
          原因经**统一翻译层**（`error.${code}`）翻译 —— 后端只回错误码，
          界面文案一律来自 11 语言 `error` 段。此处**不得**直接渲染后端字段
          （`detail` 是技术串，非中文界面会漏出中文；这是本区块此前的问题）。 */
      }
      {simulation && !simulation.simOk && (
        <p className="mt-2 text-xs" style={{ color: "var(--color-text-secondary)" }}>
          {t("stockAnalysis.simulation.autoFailReason")}
          {": "}
          {translateBackendError(simulation)}
        </p>
      )}

      {simulation?.simOk && (
        <>
          <div className="mt-3 grid grid-cols-2 gap-3 md:grid-cols-4">
            <MetricCell
              label={t("stockAnalysis.simulation.survivalLabel")}
              value={`${(simulation.survivalRate ?? 0).toFixed(1)}%`}
              note={t(
                (simulation.survivalRate ?? 0) >= 70
                  ? "stockAnalysis.monte-carlo-panel.survival-rate-high"
                  : (simulation.survivalRate ?? 0) >= 40
                  ? "stockAnalysis.monte-carlo-panel.survival-rate-medium"
                  : "stockAnalysis.monte-carlo-panel.survival-rate-low",
              )}
            />
            <MetricCell
              label={t("stockAnalysis.simulation.consistencyLabel")}
              value={consistency == null ? "—" : consistency.toFixed(2)}
              note={consistency == null
                ? t("stockAnalysis.monte-carlo-panel.consistency-undetermined")
                : consistency < 0.5
                ? t("stockAnalysis.monte-carlo-panel.consistency-high")
                : consistency < 1.0
                ? t("stockAnalysis.monte-carlo-panel.consistency-acceptable")
                : t("stockAnalysis.monte-carlo-panel.consistency-environment-dependent")}
              warn={consistency == null}
            />
            <MetricCell
              label={t("stockAnalysis.simulation.bestWorstLabel")}
              value={simulation.bestScenario ?? "—"}
              note={simulation.worstScenario && simulation.worstScenario !== simulation.bestScenario
                ? `${t("stockAnalysis.simulation.worstScenarioLabel")}: ${simulation.worstScenario}`
                : undefined}
            />
            <MetricCell
              label={t("stockAnalysis.simulation.pathsLabel")}
              value={String(simulation.totalPaths ?? 0)}
            />
          </div>

          {/* 场景明细：涨跌幅按 A 股习惯着色（涨红跌绿） */}
          {simulation.scenarioResults && simulation.scenarioResults.length > 0 && (
            <div className="mt-3 overflow-x-auto">
              <table className="w-full text-xs">
                <thead>
                  <tr className="text-secondary">
                    <th className="py-1 text-left font-normal">
                      {t("stockAnalysis.simulation.colScenario")}
                    </th>
                    <th className="py-1 text-right font-normal">
                      {t("stockAnalysis.simulation.colPaths")}
                    </th>
                    <th className="py-1 text-right font-normal">
                      {t("stockAnalysis.simulation.colChange")}
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {simulation.scenarioResults.map((row, idx) => {
                    const chg = row.priceChangePct;
                    const color = chg == null
                      ? "var(--color-text-secondary)"
                      : chg >= 0
                      ? "#f5222d"
                      : "#52c41a";
                    return (
                      <tr key={row.scenario ?? idx} className="border-t" style={{ borderColor: "var(--border)" }}>
                        <td className="py-1">{row.label ?? row.scenario ?? "—"}</td>
                        <td className="py-1 text-right font-mono">{row.paths ?? "—"}</td>
                        <td className="py-1 text-right font-mono" style={{ color }}>
                          {chg == null ? "—" : `${chg >= 0 ? "+" : ""}${chg.toFixed(2)}%`}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          )}
        </>
      )}
    </div>
  );
}

function MetricCell(
  { label, value, note, warn }: { label: string; value: string; note?: string; warn?: boolean },
) {
  return (
    <div>
      <div className="text-xs text-secondary">{label}</div>
      <div className="mt-0.5 font-mono text-base">{value}</div>
      {note && (
        <div className="text-xs" style={{ color: warn ? "#faad14" : "var(--color-text-secondary)" }}>
          {note}
        </div>
      )}
    </div>
  );
}
