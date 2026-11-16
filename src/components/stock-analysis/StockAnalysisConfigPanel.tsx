import { SettingsGroup } from "@/components/common/SettingsGroup";
import { VariableControl } from "@/components/common/VariableControls";
import { invoke } from "@/lib/invoke";
import { invokePluginCommand } from "@/lib/pluginCommand";
import { getDefaultVariables } from "@/lib/stock-analysis-defaults";
import { toDbVariable } from "@/lib/workflowVariables";
import type { Variable, WorkflowTemplateInput, WorkflowTemplateResponse } from "@/types";
import { App, Button, InputNumber, Slider, Space, Tag, theme } from "antd";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

const TEMPLATE_ID = "stock-analysis";

// getDefaultVariables 的权威定义已下沉到 `@/lib/stock-analysis-defaults`（组件中立性 A 档）：
// WhatIfBacktest 与本面板共用默认值清单，跨域消费不该反向 import 设置面板组件模块。

/**
 * 变量对象字段名兼容 helper 已抽到 `@/lib/workflowVariables`（varTypeOf /
 * isSecretOf / toDbVariable）——本面板此前是唯一实现处，另有两个面板
 * （DemandDiscovery / LiteraryCreation）存在同一 bug 却各自没有 helper，
 * 故收敛为共享模块，避免再次漂移。这里改为直接 import 使用。
 */

// eslint-disable-next-line @typescript-eslint/no-empty-object-type
interface Props {}

interface ValuationParamsConfig {
  perpetualGrowth: number;
  discountRate: number;
  defaultGrowth: number;
  minGrowth: number;
  maxGrowth: number;
  forecastYears: number;
  bondYield: number;
}

// ⚠️ **等式门禁目标**：必须与后端 `ValuationParams::default()` **逐字段相等**
// （后者派生自 `astock-data::mcp_tools` 的单一真相源常量）。
// 前端引用不到 Rust 常量，此处只能手抄 ⇒ 由 `scripts/check-valuation-defaults-parity.mjs`
// 做等式校验（不一致即 CI 红），勿手工"顺手改一个"。
// 用途仅限「`get_valuation_params` 调用失败」时的兜底显示；正常路径会被后端返回值覆盖。
const DEFAULT_VALUATION_PARAMS: ValuationParamsConfig = {
  perpetualGrowth: 0.013,
  discountRate: 0.077,
  defaultGrowth: 0.12,
  minGrowth: -0.3,
  maxGrowth: 0.3,
  forecastYears: 5,
  bondYield: 4.4,
};

// `NumberControl` / `VariableControl` 已收敛到 `./VariableControls`（三份面板共用一份，
// 数值量程统一走 `inferNumberBounds` 动态推断）。

export function StockAnalysisConfigPanel(_props: Props) {
  const { message } = App.useApp();
  const { t } = useTranslation();
  const { token } = theme.useToken();
  const [template, setTemplate] = useState<WorkflowTemplateResponse | null>(null);
  const [values, setValues] = useState<Record<string, unknown>>({});
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [valuationParams, setValuationParams] = useState<ValuationParamsConfig>(DEFAULT_VALUATION_PARAMS);
  const [valuationDirty, setValuationDirty] = useState(false);
  // 折叠状态（按 tool 记）。collapsedByDefault 的分组在展开前不渲染任何控件——
  // SettingsGroup 不做虚拟化，179 个策略参数全量渲染会让设置页首屏多出数百个
  // 受控输入框（每个都是 antd InputNumber/Slider），打开就卡。
  const [expandedGroups, setExpandedGroups] = useState<Record<string, boolean>>({});

  useEffect(() => {
    let cancelled = false;
    // 并行加载 workflow 模板和估值参数
    Promise.all([
      invoke<WorkflowTemplateResponse | null>("get_workflow_template", { id: TEMPLATE_ID }),
      invokePluginCommand<ValuationParamsConfig>("stock_get_valuation_params").catch(() => DEFAULT_VALUATION_PARAMS),
    ])
      .then(async ([rsp, valuation]) => {
        if (cancelled) { return; }
        // 加载估值参数
        if (valuation) {
          setValuationParams(valuation);
        }
        if (rsp && (!rsp.variables || rsp.variables.length === 0)) {
          // Initial load: if template has no variables, init with defaults and save back
          // 写回前转成后端 snake_case（后端 Variable 无 camelCase 注解，否则 serde 解析失败）
          const defaults = getDefaultVariables().map(toDbVariable);
          const input: WorkflowTemplateInput = {
            name: rsp.name,
            description: rsp.description,
            icon: rsp.icon,
            tags: rsp.tags,
            triggerConfig: rsp.triggerConfig,
            nodes: rsp.nodes,
            edges: rsp.edges,
            inputSchema: rsp.inputSchema,
            outputSchema: rsp.outputSchema,
            variables: defaults,
            errorConfig: rsp.errorConfig,
          };
          invoke<boolean>("update_workflow_template", { id: TEMPLATE_ID, input }).catch(() => {});
          rsp.variables = defaults;
        }
        if (rsp) {
          setTemplate(rsp);
          const map: Record<string, unknown> = {};
          for (const v of rsp.variables) { map[v.name] = v.value; }
          setValues(map);
        } else {
          // Template not found (browser mode), render with defaults directly
          const defaults = getDefaultVariables();
          const map: Record<string, unknown> = {};
          for (const v of defaults) { map[v.name] = v.value; }
          setValues(map);
        }
      })
      .catch(() => {
        if (!cancelled) { message.error(t("stockAnalysis.settings.loadFailed")); }
      })
      .finally(() => {
        if (!cancelled) { setLoading(false); }
      });
    return () => {
      cancelled = true;
    };
  }, [t, message]);

  // tool → parameter groups
  //
  // 显式标注返回类型：部分分组带 `collapsedByDefault`（2026-09-12 批量补齐的
  // 策略参数分组）。若不标注，TS 会把数组推断成「带该属性 / 不带该属性」的
  // 联合类型，渲染处访问 `g.collapsedByDefault` 直接报错。
  const toolGroups = useMemo<
    // `note` 是分组级说明文案的 **i18n 键**（不是裸文本）：有些组的参数语义会随决策口径换代而变，
    // 不写明就会被读成「改了没生效」。见 `portfolio_mgr_horizon` 组那条。
    { tool: string; label: string; vars: Variable[]; collapsedByDefault?: boolean; note?: string }[]
  >(() => {
    const allVars = template?.variables ?? getDefaultVariables();
    const varMap: Record<string, Variable> = {};
    for (const v of allVars) { varMap[v.name] = v; }

    const resolve = (names: string[]) => names.map((n) => varMap[n]).filter(Boolean);

    return [
      {
        tool: "compute_scoring",
        label: t("stockAnalysis.settings.group.scoring"),
        vars: resolve([
          "scoring_trend",
          "scoring_deviation",
          "scoring_macd",
          "scoring_volume",
          "scoring_rsi",
          "scoring_support",
          "scoring_boll",
        ]),
      },
      {
        tool: "compute_valuation",
        label: t("stockAnalysis.settings.group.value"),
        vars: resolve([
          "value_dcf_growth_rate",
          "value_dcf_perpetual_rate",
          "value_dcf_discount_rate",
          "value_moat_threshold",
          "value_fscore_buy",
          "value_safety_margin",
        ]),
      },
      {
        tool: "compute_portfolio_risk",
        label: t("stockAnalysis.settings.group.pos"),
        vars: resolve([
          "pos_max_single_pct",
          "pos_max_total",
          "pos_max_sector_pct",
        ]),
      },
      {
        tool: "calcs",
        label: t("stockAnalysis.settings.group.riskModel"),
        vars: resolve([
          "var_confidence",
          "kelly_fraction",
          "kelly_min_win_rate",
          "kelly_min_odds",
          "risk_free_rate",
          "outlier_method",
          "outlier_threshold",
          "min_confidence",
          "risk_max_drawdown_limit",
          "risk_max_daily_loss_pct",
          "risk_correlation_lookback_days",
          "risk_sharpe_annualization",
          "risk_kelly_heavy_threshold",
          "risk_kelly_medium_threshold",
        ]),
      },
      {
        tool: "portfolio_mgr_prior",
        label: t("stockAnalysis.settings.group.portfolioMgrPrior"),
        vars: resolve([
          "regime_prior_bull",
          "regime_prior_sideways",
          "regime_prior_bear",
        ]),
      },
      {
        tool: "portfolio_mgr_fusion",
        label: t("stockAnalysis.settings.group.portfolioMgrFusion"),
        vars: resolve(["trader_cap_min_weight"]),
      },
      {
        tool: "portfolio_mgr_action",
        label: t("stockAnalysis.settings.group.portfolioMgrAction"),
        vars: resolve([
          "action_buy_threshold",
          "action_increase_threshold",
          "action_hold_threshold",
          "action_watch_threshold",
          "action_reduce_threshold",
          "pos_buy_min",
          "pos_increase_min",
          "pos_cap_extreme",
          "pos_cap_high",
          "pos_cap_mid",
        ]),
      },
      {
        tool: "portfolio_mgr_risk",
        label: t("stockAnalysis.settings.group.portfolioMgrRisk"),
        vars: resolve([
          "risk_debt_extreme",
          "risk_vol_extreme",
          "risk_sharpe_extreme",
          "risk_vol_high",
          "risk_dd_high",
          "risk_roe_high",
          "risk_debt_high",
          "risk_vol_low",
          "risk_sharpe_low",
          "risk_dd_low",
          "risk_roe_low",
          "risk_debt_low",
          "risk_growth_low",
        ]),
      },
      {
        tool: "portfolio_mgr_cost",
        label: t("stockAnalysis.settings.group.portfolioMgrCost"),
        vars: resolve(["cost_pct"]),
      },
      {
        // 〇-B v2 第 3 条：逐周期止损/止盈档位。默认值与兜底在 `portfolio-mgr.rhai`，
        // 变量定义在 `seed_variables.rs`，登记在 `PORTFOLIO_MGR_TUNABLE_PARAMS` ⇒ 反思可建议。
        // ⚠ 注释必须在 `tool:` 之前：`StockAnalysisConfigPanel.test.tsx` 的分组解析器按
        //   「label 行的下一行就是 vars」匹配，注释插在 label 与 vars 之间会让整组**不被识别**
        //   ⇒ 表现为「权威源登记了却没有任何面板分组暴露」（2026-09-29 实测踩中）。
        tool: "portfolio_mgr_horizon",
        label: t("stockAnalysis.settings.group.portfolioMgrHorizon"),
        vars: resolve([
          "sl_pct_ultra_short",
          "tp_pct_ultra_short",
          "sl_pct_short",
          "tp_pct_short",
          "sl_pct_mid",
          "tp_pct_mid",
          "sl_pct_long",
          "tp_pct_long",
          // 逐档先验收缩强度 κ（Phase C）：与上面八项同组 —— 同为「反思可建议」的策略参数。
          "horizon_prior_kappa",
          // 波动率档位乘数（Phase D）：与八项百分比同组 —— 百分比是 σ 不可得时的兜底档。
          "stop_vol_mult",
          "take_profit_vol_mult",
          // 风险预算 R（Phase D-2）：仓位主口径，替代经验周期乘数。
          "risk_budget_pct",
        ]),
        // 说明放在 vars **之后**：本文件上方的解析器陷阱（注释/属性插在 label 与 vars 之间
        // 会让整组不被识别）同样适用于 JSX 属性顺序 —— 别把它挪回 label 旁边。
        note: "stockAnalysis.settings.note.portfolioMgrHorizon",
      },
      {
        tool: "signals",
        label: t("stockAnalysis.settings.group.signals"),
        vars: resolve([
          "signal_ma_fast",
          "signal_ma_slow",
          "signal_breakout_volume_mult",
        ]),
      },
      {
        tool: "recommender",
        label: t("stockAnalysis.settings.group.recommender"),
        vars: resolve([
          "reco_trend_enabled",
          "reco_reversion_enabled",
          "reco_value_enabled",
          "reco_capital_enabled",
          "reco_watchlist_enabled",
          "reco_min_confidence",
          "reco_stop_vol_mult",
          "reco_target_vol_mult",
          "reco_risk_budget_pct",
          "reco_round_trip_cost_pct",
          "reco_conf_sensitivity",
          "reco_ic_gate",
          "decision_max_history_per_stock",
        ]),
      },
      {
        // v137（裁定 2「先加设施再接线」，PLAN §一○五）：这一组 8 个变量此前**只有声明 + 面板能改**，
        // 全仓零消费者 ⇒ 改值对评分零影响。接入后按**两域**拆成两组，作用范围写在 note 里：
        //   · 窗口域（几根 bar）＝**只作用日线链** `t-scoring`；四档的窗口由各自持有期推导
        //     （`ScaleWindowPlan`），接进档侧会把四档焊成同一份 ⇒ 工具侧对这种接线直接报错。
        //   · 阈值域（比值/倍数）＝**全链生效**（§九十六(2)：阈值与 bar 的日历跨度无关）。
        // ⚠ 注释必须在 `tool:` 之前（本文件的分组解析器按「label 行的下一行就是 vars」匹配）。
        tool: "technical_indicators_daily",
        label: t("stockAnalysis.settings.group.indicatorsDailyWindows"),
        vars: resolve([
          "macd_fast",
          "macd_slow",
          "macd_signal",
          "boll_period",
          "volume_lookback",
        ]),
        // 说明必须放 vars **之后**（同上面 portfolio_mgr_horizon 那条陷阱：label 与 vars 之间
        // 插任何东西都会让整组不被分组解析器识别 ⇒ 表现为「登记了但面板没有分组暴露」）。
        note: "stockAnalysis.settings.note.indicatorsDailyWindows",
      },
      {
        tool: "technical_indicators_shared",
        label: t("stockAnalysis.settings.group.indicatorsThresholds"),
        vars: resolve([
          "boll_stddev",
          "volume_surge_ratio",
          "volume_shrink_ratio",
        ]),
        note: "stockAnalysis.settings.note.indicatorsThresholds",
      },
      {
        tool: "recommender_strategies",
        label: t("stockAnalysis.settings.group.strategyParams"),
        vars: resolve([
          "trend_kline_limit",
          "trend_amount_ratio_min",
          // 趋势策略入池门槛与共享乘数（2026-09-12 补齐定义）。
          // 这些 key 一直被 trend.rs 以 read_f64 读取并带硬编码默认值，但此前既不在
          // DB 变量表、也不在本 resolve 白名单中 → 面板上完全不可见、不可调。
          // 其中 trend_high_20_threshold 直接构成超短线入池条件
          // （现价 ≥ 5 日高 × 该比例），调不了就无法自查「超短线拿不到候选」。
          "trend_high_20_threshold",
          "trend_short_ma20_tolerance",
          "trend_ma60_threshold",
          "trend_high_60_threshold",
          "trend_entry_tightness",
          "trend_stop_mult",
          "trend_target_mult",
          "trend_position_adj",
          "trend_conf_consistency",
          "trend_conf_signal",
          "trend_conf_market",
          "rev_rsi_short_max",
        ]),
      },
      {
        tool: "strategy_gates",
        label: t("stockAnalysis.settings.group.strategyGates"),
        // 2026-09-12 批量补齐：这些 key 一直被各策略以 read_f64 读取并带
        // 硬编码默认值，但既不在 DB 变量表、也不在本白名单 ⇒ 面板上完全不可见。
        // 门槛与过滤条件：32 项。决定「是否入池」。超短线拿不到候选时先查这里：cap_ultra_short_turnover_min / cap_kline_vol_ratio_min 过严，或 val_ultra_short_pe_max 偏低，都会让候选被静默剔除。
        collapsedByDefault: true,
        vars: resolve([
          "cap_kline_mom_5_max",
          "cap_kline_mom_5_min",
          "cap_kline_vol_ratio_min",
          "cap_long_main_inflow_min",
          "cap_long_nb_ratio_min",
          "cap_mid_main_inflow_min",
          "cap_mid_nb_ratio_min",
          "cap_short_main_inflow_min",
          "cap_short_turnover_min",
          "cap_ultra_short_dt_net_min",
          "cap_ultra_short_turnover_min",
          "rev_dd_min",
          "rev_min_divergence_strength",
          "rev_rsi_mid_max",
          "rev_rsi_mid_max_divergence",
          "rev_rsi_mid_period",
          "rev_rsi_period",
          "rev_rsi_short_max_divergence",
          "serenity_growth_exempt_pct",
          "serenity_max_12m_gain_pct",
          "serenity_max_3m_gain_pct",
          "serenity_max_debt_ratio",
          "serenity_max_pb",
          "serenity_max_pe",
          "serenity_min_gross_margin",
          "serenity_min_revenue_growth",
          "val_long_pb_max",
          "val_long_pe_max",
          "val_mid_pb_max",
          "val_mid_pe_max",
          "val_short_pe_max",
          "val_ultra_short_pe_max",
        ]),
      },
      {
        tool: "strategy_confidence",
        label: t("stockAnalysis.settings.group.strategyConfidence"),
        // 2026-09-12 批量补齐：这些 key 一直被各策略以 read_f64 读取并带
        // 硬编码默认值，但既不在 DB 变量表、也不在本白名单 ⇒ 面板上完全不可见。
        // 置信度分量与权重：27 项。各策略的置信度分量权重与加成，决定打分尺度。注意 reco_min_confidence 是全局闸门（在「推荐器」组），这里调的是「分数怎么算出来」。
        collapsedByDefault: true,
        vars: resolve([
          "cap_conf_consistency",
          "cap_conf_direction",
          "cap_conf_market",
          "cap_conf_signal",
          "rev_conf_base",
          "rev_conf_consistency",
          "rev_conf_direction",
          "rev_conf_market",
          "rev_conf_signal",
          "rev_divergence_bonus",
          "rev_pattern_bonus",
          "syn_conf_base",
          "syn_conf_consistency",
          "syn_conf_direction",
          "syn_conf_market",
          "syn_conf_signal",
          "syn_min_confidence",
          "val_conf_consistency",
          "val_conf_direction",
          "val_conf_market",
          "val_conf_signal",
          "wl_conf_base",
          "wl_conf_consistency",
          "wl_conf_direction",
          "wl_conf_market",
          "wl_conf_signal",
          "wl_min_confidence",
        ]),
      },
      {
        tool: "strategy_position",
        label: t("stockAnalysis.settings.group.strategyPosition"),
        // 2026-09-12 批量补齐：这些 key 一直被各策略以 read_f64 读取并带
        // 硬编码默认值，但既不在 DB 变量表、也不在本白名单 ⇒ 面板上完全不可见。
        // 仓位与价格区间：107 项。入场区间 / 止损 / 目标 / 基准仓位，按策略 × 周期细分。默认值与代码常量逐一对齐，改之前请确认影响面。
        collapsedByDefault: true,
        vars: resolve([
          "cap_long_base_pos",
          "cap_long_entry_high",
          "cap_long_entry_low",
          "cap_long_stop",
          "cap_long_target",
          "cap_mid_base_pos",
          "cap_mid_entry_high",
          "cap_mid_entry_low",
          "cap_mid_stop",
          "cap_mid_target",
          "cap_short_base_pos",
          "cap_short_entry_high",
          "cap_short_entry_low",
          "cap_short_stop",
          "cap_short_target",
          "cap_ultra_short_base_pos",
          "cap_ultra_short_entry_high",
          "cap_ultra_short_entry_low",
          "cap_ultra_short_stop",
          "cap_ultra_short_target",
          "rev_avg_amount_mult",
          "rev_mid_base_pos",
          "rev_mid_entry_high",
          "rev_mid_entry_low",
          "rev_mid_stop",
          "rev_mid_target",
          "rev_short_base_pos",
          "rev_short_entry_high",
          "rev_short_entry_low",
          "rev_short_stop",
          "rev_short_target",
          "serenity_entry_range",
          "serenity_stop_mult",
          "serenity_target_mult",
          "syn_long_base_pos",
          "syn_long_entry_high",
          "syn_long_entry_low",
          "syn_long_stop",
          "syn_long_target",
          "syn_mid_base_pos",
          "syn_mid_entry_high",
          "syn_mid_entry_low",
          "syn_mid_stop",
          "syn_mid_target",
          "syn_short_base_pos",
          "syn_short_entry_high",
          "syn_short_entry_low",
          "syn_short_stop",
          "syn_short_target",
          "syn_ultra_short_base_pos",
          "syn_ultra_short_entry_high",
          "syn_ultra_short_entry_low",
          "syn_ultra_short_stop",
          "syn_ultra_short_target",
          "val_long_base_pos",
          "val_long_entry_high",
          "val_long_entry_low",
          "val_long_ma60_mult",
          "val_long_stop",
          "val_long_target",
          "val_mid_base_pos",
          "val_mid_entry_high",
          "val_mid_entry_low",
          "val_mid_stop",
          "val_mid_target",
          "val_short_base_pos",
          "val_short_entry_high",
          "val_short_entry_low",
          "val_short_ma_mult",
          "val_short_stop",
          "val_short_target",
          "val_ultra_short_base_pos",
          "val_ultra_short_entry_high",
          "val_ultra_short_entry_low",
          "val_ultra_short_ma_mult",
          "val_ultra_short_stop",
          "val_ultra_short_target",
          "wl_long_base_pos",
          "wl_long_entry_high",
          "wl_long_entry_low",
          "wl_long_stop",
          "wl_long_target",
          "wl_mid_base_pos",
          "wl_mid_entry_high",
          "wl_mid_entry_low",
          "wl_mid_stop",
          "wl_mid_target",
          "wl_short_base_pos",
          "wl_short_entry_high",
          "wl_short_entry_low",
          "wl_short_stop",
          "wl_short_target",
          "wl_ultra_short_base_pos",
          "wl_ultra_short_entry_high",
          "wl_ultra_short_entry_low",
          "wl_ultra_short_stop",
          "wl_ultra_short_target",
        ]),
      },
      {
        tool: "strategy_data",
        label: t("stockAnalysis.settings.group.strategyData"),
        // 2026-09-12 批量补齐：这些 key 一直被各策略以 read_f64 读取并带
        // 硬编码默认值，但既不在 DB 变量表、也不在本白名单 ⇒ 面板上完全不可见。
        // 数据窗口与限流：13 项。K 线读取上限、最少条数、均线与均量周期。
        collapsedByDefault: true,
        vars: resolve([
          "rev_avg_amount_period",
          "rev_dd_period",
          "rev_divergence_lookback",
          "rev_kline_limit",
          "rev_min_kline_len",
          "val_long_ma_period",
          "val_short_ma_period",
          "val_short_min_kline_len",
          "val_ultra_short_ma_period",
          "val_ultra_short_min_kline_len",
        ]),
      },
      {
        tool: "workflow_runtime",
        label: t("stockAnalysis.settings.group.workflow"),
        vars: resolve([
          "analysis_depth",
          "debate_rounds",
          "max_concurrent",
        ]),
      },
      {
        tool: "agent_executor",
        label: t("stockAnalysis.settings.group.agentRuntime"),
        vars: resolve([
          "agent_timeout_secs",
          "agent_retry_max",
        ]),
      },
      {
        tool: "tool_executor",
        label: t("stockAnalysis.settings.group.toolRuntime"),
        vars: resolve([
          "tool_timeout_secs",
          "tool_retry_max",
          "kline_limit",
          "news_limit",
        ]),
      },
      {
        tool: "monitor",
        label: t("stockAnalysis.settings.group.monitor"),
        vars: resolve(["monitor_poll_interval_secs"]),
      },
      {
        tool: "workflow",
        label: t("stockAnalysis.settings.group.dryRun"),
        vars: resolve(["analysis_dry_run"]),
      },
      {
        tool: "simulation",
        label: t("stockAnalysis.settings.group.simulation"),
        vars: resolve([
          "mc_default_price",
          "mc_default_return",
          "mc_default_volatility",
          "mc_default_days",
          "mc_default_simulations",
        ]),
      },
    ].filter((g) => g.vars.length > 0);
  }, [template, t]);

  const handleChange = (name: string, val: unknown) => {
    setValues((prev) => ({ ...prev, [name]: val }));
  };

  const handleValuationChange = (key: keyof ValuationParamsConfig, value: number) => {
    setValuationParams((prev) => ({ ...prev, [key]: value }));
    setValuationDirty(true);
  };

  const handleSave = async () => {
    if (!template) { return; }
    setSaving(true);
    const updatedVars = template.variables.map((v) => ({ ...v, value: values[v.name] ?? v.value }));
    const input: WorkflowTemplateInput = {
      name: template.name,
      description: template.description,
      icon: template.icon,
      tags: template.tags,
      triggerConfig: template.triggerConfig,
      nodes: template.nodes,
      edges: template.edges,
      inputSchema: template.inputSchema,
      outputSchema: template.outputSchema,
      variables: updatedVars,
      errorConfig: template.errorConfig,
      toolDefs: template.toolDefs,
    };
    try {
      // 并行保存 workflow 模板和估值参数
      const results = await Promise.allSettled([
        invoke<boolean>("update_workflow_template", { id: TEMPLATE_ID, input }),
        valuationDirty
          ? invokePluginCommand<boolean>("stock_save_valuation_params", { params: valuationParams })
          : Promise.resolve(true),
      ]);
      const [templateResult, valuationResult] = results;
      if (templateResult.status === "rejected") {
        throw templateResult.reason;
      }
      if (valuationResult.status === "rejected") {
        console.warn("[StockAnalysisConfigPanel] valuation params save failed:", valuationResult.reason);
      } else {
        setValuationDirty(false);
      }
      message.success(t("stockAnalysis.settings.saveSuccess"));
    } catch (e) {
      console.error("[StockAnalysisConfigPanel] save failed:", e, { input });
      message.error(t("stockAnalysis.settings.saveFailed", { error: String(e) }));
    } finally {
      setSaving(false);
    }
  };

  if (loading) {
    return (
      <div style={{ textAlign: "center", padding: 24, color: token.colorTextQuaternary }}>{t("common.loading")}</div>
    );
  }

  const rowStyle = { padding: "4px 0" };

  const handleOptimize = async () => {
    setSaving(true);
    try {
      const weights = await invoke<Record<string, unknown>>("optimize_scoring_weights");
      if (weights) {
        const map: Record<string, number> = {
          scoring_trend: weights.trendWeight as number,
          scoring_deviation: weights.deviationWeight as number,
          scoring_macd: weights.macdWeight as number,
          scoring_volume: weights.volumeWeight as number,
          scoring_rsi: weights.rsiWeight as number,
          scoring_support: weights.supportWeight as number,
        };
        setValues((prev) => ({ ...prev, ...map }));
        message.success(t("stockAnalysis.settings.optimize.success"));
      }
    } catch {
      message.error(t("stockAnalysis.settings.optimize.failed"));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="flex flex-col gap-3">
      <div className="flex justify-end gap-2">
        {valuationDirty && <Tag color="orange">{t("stockAnalysis.settings.valuation.unsavedChanges")}</Tag>}
        <Button size="small" loading={saving} onClick={handleOptimize}>
          {t("stockAnalysis.settings.optimize.btn")}
        </Button>
      </div>

      {/* 估值模型参数配置 */}
      <SettingsGroup
        title={
          <Space size={4}>
            <span>{t("stockAnalysis.settings.group.valuationModel")}</span>
            <Tag className="text-xs m-0" color="blue">📈 compute_valuation</Tag>
          </Space>
        }
      >
        <div className="sacp-vars">
          {([
            // 2026-09-23: hint（建议值）同步到新口径 —— 旧 hint `0.03 (3%)` / `0.10 (10%)`
            // 是校准前的漂移值，其中永续增长率 3% 现已**越过** `g_terminal ≤ r_f` 硬约束
            // （r_f = 1.7%）⇒ 照 hint 填会得到一个会被守卫钳回并报信号的非法配置。
            ["perpetualGrowth", t("stockAnalysis.settings.valuation.perpetualGrowth"), "0.015 (1.5%)", 0, 0.20, 0.01],
            ["discountRate", t("stockAnalysis.settings.valuation.discountRate"), "0.077 (7.7%)", 0.01, 0.30, 0.01],
            ["defaultGrowth", t("stockAnalysis.settings.valuation.defaultGrowth"), "0.08 (8%)", 0.01, 0.50, 0.01],
            ["minGrowth", t("stockAnalysis.settings.valuation.minGrowth"), "0.02 (2%)", 0, 0.20, 0.01],
            ["maxGrowth", t("stockAnalysis.settings.valuation.maxGrowth"), "0.30 (30%)", 0.05, 1.00, 0.01],
            ["forecastYears", t("stockAnalysis.settings.valuation.forecastYears"), "5", 1, 15, 1],
            ["bondYield", t("stockAnalysis.settings.valuation.bondYield"), "4.4", 1.0, 10.0, 0.1],
          ] as const).map(([key, label, hint, min, max, step]) => (
            <div key={key} style={rowStyle} className="flex items-center justify-between sacp-row">
              <span style={{ fontSize: 13, color: token.colorText }}>
                {label}
                <span style={{ color: token.colorTextTertiary, marginLeft: 8, fontSize: 12 }}>({hint})</span>
              </span>
              <span style={{ display: "inline-flex", alignItems: "center", gap: 8, flexShrink: 0, marginLeft: 16 }}>
                <Slider
                  min={min}
                  max={max}
                  step={step}
                  style={{ width: 120 }}
                  value={valuationParams[key]}
                  onChange={(v) =>
                    handleValuationChange(key, v as number)}
                />
                <InputNumber
                  size="small"
                  style={{ width: 80 }}
                  min={min}
                  max={max}
                  step={step}
                  value={valuationParams[key]}
                  onChange={(v) =>
                    v != null && handleValuationChange(key, v)}
                />
              </span>
            </div>
          ))}
        </div>
      </SettingsGroup>

      {toolGroups.map((g) => {
        const collapsible = g.collapsedByDefault === true;
        const expanded = expandedGroups[g.tool] === true;
        const open = !collapsible || expanded;
        return (
          <SettingsGroup
            key={g.tool}
            title={
              <Space size={4}>
                <span>{g.label}</span>
                <Tag className="text-xs m-0" color="default">⚙️ {g.tool}</Tag>
              </Space>
            }
            extra={collapsible
              ? (
                <Button
                  size="small"
                  type="link"
                  className="!h-auto !px-1 !text-xs"
                  onClick={() => setExpandedGroups((prev) => ({ ...prev, [g.tool]: !expanded }))}
                >
                  {expanded
                    ? t("stockAnalysis.settings.collapseGroup")
                    : t("stockAnalysis.settings.expandGroup", { total: g.vars.length })}
                </Button>
              )
              : undefined}
          >
            {open && (
              <div className="sacp-vars">
                {g.note
                  ? (
                    <div style={{ fontSize: 12, color: token.colorTextSecondary, marginBottom: 8, lineHeight: 1.5 }}>
                      {t(g.note)}
                    </div>
                  )
                  : null}
                {g.vars.map((v) => (
                  <div key={v.name} style={rowStyle} className="flex items-center justify-between sacp-row">
                    <span className="sacp-var-label" style={{ fontSize: 13, color: token.colorText }}>
                      {v.description ? t(v.description) : v.name}
                    </span>
                    <span
                      style={{ display: "inline-flex", alignItems: "center", gap: 8, flexShrink: 0, marginLeft: 16 }}
                    >
                      <VariableControl v={v} value={values[v.name]} onChange={handleChange} />
                    </span>
                  </div>
                ))}
              </div>
            )}
          </SettingsGroup>
        );
      })}
      <div style={{ display: "flex", justifyContent: "flex-end", paddingTop: 8 }}>
        <Button type="primary" loading={saving} onClick={handleSave}>
          {t("stockAnalysis.settings.saveConfig")}
        </Button>
      </div>
    </div>
  );
}
