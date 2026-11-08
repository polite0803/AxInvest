// SPDX-License-Identifier: AGPL-3.0-only
//
// 股票分析模板参数默认值 —— 前端单一权威源。
// 组件中立性 A 档：由 components/stock-analysis/StockAnalysisConfigPanel 下沉至此。
// WhatIfBacktest（stock-analysis 域）与设置面板（settings 域）共用这份清单，
// 任何一方都不该为取默认值反向 import 另一方的组件模块。

import type { Variable } from "@/types";

/** Generate default parameter variable list (initial load, sync with stock-analysis workflow template v19).
 * Naming convention: snake_case, must match seed keys in `stock_analysis_setup.rs`.
 * Update this when backend defaults change to keep UI in sync with actual runtime params.
 *
 * **这是前端参数默认值的单一权威源，按名导出供其他面板复用**（如
 * `WhatIfBacktest` 的 What-If 滑块需要「参数名 → 默认值 + i18n 标签」映射）。
 * 在此处新增/修改参数名或默认值，两处面板同时生效 —— 不要再在别处复制一份清单，
 * 否则会重演「同一参数两套默认值」分叉（后端 `replay_tool_chain` 曾因此把
 * `risk_max_drawdown_limit` 默认写成 20 而此处是 15）。 */
export function getDefaultVariables(): Variable[] {
  const vars: Variable[] = [];
  const b = (name: string, val: unknown, desc: string, type: string) =>
    vars.push({ name, varType: type, value: val, description: desc, isSecret: false });
  // 分析流程
  b("analysis_depth", "standard", "stockAnalysis.configDescriptions.analysisDepth", "enum");
  // 权威源是后端 `seed_variables::DEFAULT_DEBATE_ROUNDS`（**四档化后 = 1**），此处只是
  // 「模板 variables 为空时的初始化值」，而这条路径会把值写回库里 ⇒ 必须与工厂缺省同数。
  // ⚠ 旧注释说「多轮是假多轮、与 harness serde 缺省收敛」，两句都已失效：
  //   ① v59（2026-09-21）起每轮展开独立 bull-rN/bear-rN 节点，各轮真跑（见建图注释）；
  //   ② harness 的 `default_debate_rounds()` 守护的是容器轮播次数（恒 1），不是轮数域。
  // ⚠ 2 → 1（B3 换代）：轮数语义从「每模板几轮」变为「**每档几轮**」，B3 明文 1 轮/档，
  //   代价账 `4×(1+1+1)=12` 次 LLM 调用即按此数算出（见 seed_variables 的同项文档）。
  //   存量库的自选轮数由 `DEBATE_ROUNDS_MIGRATION_VERSION` 一次性打回 1；
  //   此处若留 2，空模板路径会把 2 写回库里 ⇒ 与迁移后的值当场打架，
  //   且实测跑几轮又取决于走哪条初始化路径（同型踩坑见文件上方 max_concurrent 注释）。
  b("debate_rounds", 1, "stockAnalysis.configDescriptions.debateRounds", "number");
  // #11（2026-10-05）：12 → 8，与后端工厂缺省收敛。权威源是
  // `seed_variables.rs` 的 `max_concurrent`（2026-09-08 依 429 重试占槽实证由 3 调到 8），
  // 此处只是「模板 variables 为空时的初始化值」——而那条路径**会把这批值写回库里**
  // （见下方 `update_workflow_template` 调用），所以它写的数字必须是工厂缺省而不是随手值。
  // 现网库里的 10 是用户改过的运行时值，不受影响；引擎侧并发另有按类型分档的兜底
  // （`rt-workflow/.../engine/mod.rs`，llm/agent=3、tool/file=10），两者不是一回事。
  b("max_concurrent", 8, "stockAnalysis.configDescriptions.maxConcurrent", "number");
  // 数据源参数
  // v115（2026-10-01）：120 → 250。权威源是后端 `seed_variables::DEFAULT_ANALYST_KLINE_LIMIT`，
  // 此处只是「后端变量表取不到时的兜底」——但两者不同步会让人误判参数没生效
  // （`numberBounds.inferNumberBounds` 还会按本值推滑杆量程：120→上限 180，
  //  250→上限 380，量程小于真实值时用户一拖就把取数根数改小）。
  b("kline_limit", 250, "stockAnalysis.configDescriptions.klineLimit", "number");
  b("news_limit", 30, "stockAnalysis.configDescriptions.newsLimit", "number");
  // Agent 节点 LLM 参数。
  // 2026-10-26（模板 v189）：`agent_temperature` / `agent_max_tokens` 两格**已撤下** ——
  // 采样参数的权威在「供应商-模型 → 参数」那一格（聊天链一直读它，工作流此前不读，
  // 于是面板上设的 124672 被这里的 4096 盖掉，实测 10 个决策节点输出被截断）。
  // 在这里再放一份默认值就等于再造一个「面板能拨、代码不读」的开关。
  b("agent_timeout_secs", 600, "stockAnalysis.configDescriptions.agentTimeoutSecs", "number");
  b("agent_retry_max", 2, "stockAnalysis.configDescriptions.agentRetryMax", "number");
  // Tool 节点参数
  b("tool_timeout_secs", 30, "stockAnalysis.configDescriptions.toolTimeoutSecs", "number");
  b("tool_retry_max", 2, "stockAnalysis.configDescriptions.toolRetryMax", "number");
  // 评分权重
  b("scoring_trend", 30, "stockAnalysis.configDescriptions.scoringTrend", "number");
  b("scoring_deviation", 20, "stockAnalysis.configDescriptions.scoringDeviation", "number");
  b("scoring_macd", 15, "stockAnalysis.configDescriptions.scoringMacd", "number");
  b("scoring_volume", 15, "stockAnalysis.configDescriptions.scoringVolume", "number");
  b("scoring_rsi", 10, "stockAnalysis.configDescriptions.scoringRsi", "number");
  b("scoring_support", 10, "stockAnalysis.configDescriptions.scoringSupport", "number");
  b("scoring_boll", 5, "stockAnalysis.configDescriptions.scoringBoll", "number");
  // 规则阈值
  // 仓位限制
  b("pos_max_single_pct", 20, "stockAnalysis.configDescriptions.posMaxSinglePct", "number");
  b("pos_max_total", 10, "stockAnalysis.configDescriptions.posMaxTotal", "number");
  b("pos_max_sector_pct", 40, "stockAnalysis.configDescriptions.posMaxSectorPct", "number");
  // 估值参数（唯一权威是 `mcp_tools.rs` 的三个常量，种子按 ×100 派生，此处等值抄种子。
  // ⚠ 2026-10-25 实测此处曾停在**上一代校准**（perpetual 4 / discount 8.5，而权威是 1.3 / 7.7）
  //   ⇒ 「模板 variables 为空」那条初始化路径会把旧值写回库里。逐字段等式由
  //   `scripts/check-stock-var-defaults-parity.mjs` 守，改权威值必须同批改这三行。）
  b("value_dcf_growth_rate", 12, "stockAnalysis.configDescriptions.valueDcfGrowthRate", "number");
  b("value_dcf_perpetual_rate", 1.3, "stockAnalysis.configDescriptions.valueDcfPerpetualRate", "number");
  b("value_dcf_discount_rate", 7.7, "stockAnalysis.configDescriptions.valueDcfDiscountRate", "number");
  b("value_moat_threshold", 60, "stockAnalysis.configDescriptions.valueMoatThreshold", "number");
  b("value_fscore_buy", 7, "stockAnalysis.configDescriptions.valueFscoreBuy", "number");
  b("value_safety_margin", 20, "stockAnalysis.configDescriptions.valueSafetyMargin", "number");
  // ── 已移除的死配置（2026-09-11 全项目核实）──
  // 护城河（moat_roe_years_min / moat_avg_gross_margin_min / moat_margin_stable_std_max /
  // moat_fcf_ratio_min）与选股筛选（screener_* 7 项）此前在本处声明，但：
  //   · seed_variables.rs 从未定义 → DB 变量表里不存在 → rhai/工具读取时 present() 恒假；
  //   · 全项目 grep 无任何消费方（moat_* 对应 analysis-engine/src/value.rs 的硬编码
  //     `roe_years_above_15 >= 3`；screener_* 无读取点）；
  //   · 面板 `resolve().filter(Boolean)` 早已把它们整组滤掉，界面上从未显示。
  // 一并移除的还有 limit_up_* 6 项（crates/tools/src/tools/finance.rs 走的是 ToolNode
  // args 而非模板变量）、rev_* / val_pe_* / val_pb_* / cap_* 8 项、
  // pos_min_cash_pct / pos_max_turnover_pct / trading_price_deviation_limit。
  // 若将来要支持「可配置」，必须补齐三件套，缺一即重演「配置项空接线」：
  //   ① seed_variables.rs 定义变量；② 对应 input_mapping 同名映射；
  //   ③ 消费点从硬编码改为读取该变量。
  // 监控
  b("monitor_poll_interval_secs", 30, "stockAnalysis.configDescriptions.monitorPollIntervalSecs", "number");
  b("monitor_alert_cooldown_secs", 300, "stockAnalysis.configDescriptions.monitorAlertCooldownSecs", "number");
  b("monitor_min_severity", "info", "stockAnalysis.configDescriptions.monitorMinSeverity", "enum");
  b("monitor_channels", "in_app", "stockAnalysis.configDescriptions.monitorChannels", "string");
  // 风险/置信度
  b("min_confidence", 60, "stockAnalysis.configDescriptions.minConfidence", "number");
  b("var_confidence", 0.95, "stockAnalysis.configDescriptions.varConfidence", "number");
  b("kelly_fraction", 0.5, "stockAnalysis.configDescriptions.kellyFraction", "number");
  b("kelly_min_win_rate", 0.4, "stockAnalysis.configDescriptions.kellyMinWinRate", "number");
  b("kelly_min_odds", 1.0, "stockAnalysis.configDescriptions.kellyMinOdds", "number");
  b("risk_free_rate", 0.03, "stockAnalysis.configDescriptions.riskFreeRate", "number");
  // 市况先验（prior 由市况方向派生；可被反思/演进优化调整）
  b("regime_prior_bull", 0.55, "stockAnalysis.configDescriptions.regimePriorBull", "number");
  b("regime_prior_sideways", 0.5, "stockAnalysis.configDescriptions.regimePriorSideways", "number");
  b("regime_prior_bear", 0.45, "stockAnalysis.configDescriptions.regimePriorBear", "number");
  // 因子融合门（f7 权重低于该值时，交易员看空信号不再封顶后验概率）
  // 2026-09-11: portfolio-mgr.rhai 早有 `present(trader_cap_min_weight)` 守卫，
  // 但变量表从未定义该名 → present() 恒假、恒走硬编码 0.08。本次补齐三件套。
  b(
    "trader_cap_min_weight",
    0.08,
    "stockAnalysis.configDescriptions.traderCapMinWeight",
    "number",
  );
  // 交易成本率（凯利仓位修正按 (1-成本率) 折损预期收益）
  // 2026-09-11: 权威源 PORTFOLIO_MGR_TUNABLE_PARAMS 一直登记着它、input_mapping
  // 也一直在注入它，但面板里 **零出现**（既无此声明也无任何 resolve() 分组）
  // —— 是全清单唯一没有 UI 入口的参数，用户想改只能改 DB。本次补齐末环。
  b("cost_pct", 0.003, "stockAnalysis.configDescriptions.costPct", "number");
  // portfolio-mgr 决策阈值（修复 D7/D8: 与 portfolio-mgr.rhai 顶部可配置参数一一对应）
  b("action_buy_threshold", 0.63, "stockAnalysis.configDescriptions.actionBuyThreshold", "number");
  b("action_increase_threshold", 0.53, "stockAnalysis.configDescriptions.actionIncreaseThreshold", "number");
  b("action_hold_threshold", 0.5, "stockAnalysis.configDescriptions.actionHoldThreshold", "number");
  b("action_watch_threshold", 0.38, "stockAnalysis.configDescriptions.actionWatchThreshold", "number");
  b("action_reduce_threshold", 0.30, "stockAnalysis.configDescriptions.actionReduceThreshold", "number");
  b("pos_buy_min", 15.0, "stockAnalysis.configDescriptions.posBuyMin", "number");
  b("pos_increase_min", 10.0, "stockAnalysis.configDescriptions.posIncreaseMin", "number");
  b("pos_cap_extreme", 10.0, "stockAnalysis.configDescriptions.posCapExtreme", "number");
  b("pos_cap_high", 35.0, "stockAnalysis.configDescriptions.posCapHigh", "number");
  b("pos_cap_mid", 50.0, "stockAnalysis.configDescriptions.posCapMid", "number");
  b("risk_debt_extreme", 85.0, "stockAnalysis.configDescriptions.riskDebtExtreme", "number");
  b("risk_vol_extreme", 60.0, "stockAnalysis.configDescriptions.riskVolExtreme", "number");
  b("risk_sharpe_extreme", -1.5, "stockAnalysis.configDescriptions.riskSharpeExtreme", "number");
  b("risk_vol_high", 40.0, "stockAnalysis.configDescriptions.riskVolHigh", "number");
  b("risk_dd_high", 45.0, "stockAnalysis.configDescriptions.riskDdHigh", "number");
  b("risk_roe_high", 5.0, "stockAnalysis.configDescriptions.riskRoeHigh", "number");
  b("risk_debt_high", 65.0, "stockAnalysis.configDescriptions.riskDebtHigh", "number");
  b("risk_vol_low", 25.0, "stockAnalysis.configDescriptions.riskVolLow", "number");
  b("risk_sharpe_low", 0.5, "stockAnalysis.configDescriptions.riskSharpeLow", "number");
  b("risk_dd_low", 30.0, "stockAnalysis.configDescriptions.riskDdLow", "number");
  b("risk_roe_low", 8.0, "stockAnalysis.configDescriptions.riskRoeLow", "number");
  b("risk_debt_low", 55.0, "stockAnalysis.configDescriptions.riskDebtLow", "number");
  b("risk_growth_low", 3.0, "stockAnalysis.configDescriptions.riskGrowthLow", "number");
  b("outlier_method", "zscore", "stockAnalysis.configDescriptions.outlierMethod", "enum");
  b("outlier_threshold", 2.0, "stockAnalysis.configDescriptions.outlierThreshold", "number");
  b("risk_max_drawdown_limit", 15, "stockAnalysis.configDescriptions.riskMaxDrawdownLimit", "number");
  b("risk_max_daily_loss_pct", 3, "stockAnalysis.configDescriptions.riskMaxDailyLossPct", "number");
  b("risk_correlation_lookback_days", 60, "stockAnalysis.configDescriptions.riskCorrelationLookbackDays", "number");
  // 行业财务基线参考股票代码（stock_analysis_setup 中 t-baseline-* 节点使用）
  // 信号检测（signals.rs detect_ma_cross / detect_breakout）
  b("signal_ma_fast", 5, "stockAnalysis.configDescriptions.signalMaFast", "number");
  b("signal_ma_slow", 20, "stockAnalysis.configDescriptions.signalMaSlow", "number");
  b("signal_breakout_volume_mult", 1.5, "stockAnalysis.configDescriptions.signalBreakoutVolumeMult", "number");
  // 推荐器策略开关（recommender/strategies）
  b("reco_trend_enabled", true, "stockAnalysis.configDescriptions.recoTrendEnabled", "boolean");
  b("reco_reversion_enabled", true, "stockAnalysis.configDescriptions.recoReversionEnabled", "boolean");
  b("reco_value_enabled", true, "stockAnalysis.configDescriptions.recoValueEnabled", "boolean");
  b("reco_capital_enabled", true, "stockAnalysis.configDescriptions.recoCapitalEnabled", "boolean");
  b("reco_watchlist_enabled", true, "stockAnalysis.configDescriptions.recoWatchlistEnabled", "boolean");
  // 权威是种子 `reco_min_confidence = 50`（60 会滤掉 value/reversion/watchlist 的超短线产出，
  // 见 seed_variables.rs 该块的注释）；此处曾停在 60 ⇒ 空模板路径会把门槛改回 60。
  b("reco_min_confidence", 50, "stockAnalysis.configDescriptions.recoMinConfidence", "number");
  b("reco_stop_vol_mult", 1.2, "stockAnalysis.configDescriptions.recoStopVolMult", "number");
  b("reco_target_vol_mult", 2.0, "stockAnalysis.configDescriptions.recoTargetVolMult", "number");
  b("reco_risk_budget_pct", 1.5, "stockAnalysis.configDescriptions.recoRiskBudgetPct", "number");
  b("reco_round_trip_cost_pct", 0.6, "stockAnalysis.configDescriptions.recoRoundTripCostPct", "number");
  b("reco_conf_sensitivity", 1.0, "stockAnalysis.configDescriptions.recoConfSensitivity", "number");
  // 权威是种子 `DEFAULT_RECO_IC_GATE = "on"`（v172 起闭环生效，且同批 `force_variable_value`
  // 覆写过存量库的 "shadow"）；此处曾停在 "shadow" ⇒ 空模板路径会把闸退回影子模式。
  b("reco_ic_gate", "on", "stockAnalysis.configDescriptions.recoIcGate", "string");
  // 决策回溯（decision_tracker.rs）
  b("decision_max_history_per_stock", 50, "stockAnalysis.configDescriptions.decisionMaxHistoryPerStock", "number");
  // 技术指标周期（indicators.rs IndicatorConfig）
  b("macd_fast", 12, "stockAnalysis.configDescriptions.macdFast", "number");
  b("macd_slow", 26, "stockAnalysis.configDescriptions.macdSlow", "number");
  b("macd_signal", 9, "stockAnalysis.configDescriptions.macdSignal", "number");
  b("boll_period", 20, "stockAnalysis.configDescriptions.bollPeriod", "number");
  b("boll_stddev", 2.0, "stockAnalysis.configDescriptions.bollStddev", "number");
  b("volume_lookback", 5, "stockAnalysis.configDescriptions.volumeLookback", "number");
  b("volume_surge_ratio", 1.5, "stockAnalysis.configDescriptions.volumeSurgeRatio", "number");
  b("volume_shrink_ratio", 0.7, "stockAnalysis.configDescriptions.volumeShrinkRatio", "number");
  // 推荐器参数（recommender/strategies）
  b("trend_kline_limit", 250, "stockAnalysis.configDescriptions.trendKlineLimit", "number");
  b("trend_amount_ratio_min", 0.8, "stockAnalysis.configDescriptions.trendAmountRatioMin", "number");
  b("rev_rsi_short_max", 35, "stockAnalysis.configDescriptions.revRsiShortMax", "number");
  // 交易决策（trading.rs）
  // 风险模型扩展（risk.rs）
  b("risk_sharpe_annualization", 252, "stockAnalysis.configDescriptions.riskSharpeAnnualization", "number");
  b("risk_kelly_heavy_threshold", 0.25, "stockAnalysis.configDescriptions.riskKellyHeavyThreshold", "number");
  b("risk_kelly_medium_threshold", 0.1, "stockAnalysis.configDescriptions.riskKellyMediumThreshold", "number");
  // 风险组合（compute_portfolio_risk / compute_scoring / compute_valuation）
  b("val_pe_low", 15, "stockAnalysis.configDescriptions.valPeLow", "number");
  b("val_pe_high", 50, "stockAnalysis.configDescriptions.valPeHigh", "number");
  b("val_pb_low", 1.0, "stockAnalysis.configDescriptions.valPbLow", "number");
  b("val_pb_high", 6.0, "stockAnalysis.configDescriptions.valPbHigh", "number");
  b("risk_hhi_concentrated", 0.25, "stockAnalysis.configDescriptions.riskHhiConcentrated", "number");
  // 凯利公式默认值
  b("kelly_default_win_rate", 0.5, "stockAnalysis.configDescriptions.kellyDefaultWinRate", "number");
  b("kelly_default_avg_win", 0.05, "stockAnalysis.configDescriptions.kellyDefaultAvgWin", "number");
  b("kelly_default_avg_loss", 0.05, "stockAnalysis.configDescriptions.kellyDefaultAvgLoss", "number");
  // 技术指标周期
  b("atr_period", 14, "stockAnalysis.configDescriptions.atrPeriod", "number");
  b("kdj_n", 9, "stockAnalysis.configDescriptions.kdjN", "number");
  // 数据清洗
  b("fill_missing_method", "forward", "stockAnalysis.configDescriptions.fillMissingMethod", "enum");
  // 突破检测
  b("breakout_volume_threshold", 1.5, "stockAnalysis.configDescriptions.breakoutVolumeThreshold", "number");
  // 业绩超预期分级阈值
  b("earnings_th_huge_pos", 50, "stockAnalysis.configDescriptions.earningsThHugePos", "number");
  b("earnings_th_strong_pos", 20, "stockAnalysis.configDescriptions.earningsThStrongPos", "number");
  b("earnings_th_mild_pos", 5, "stockAnalysis.configDescriptions.earningsThMildPos", "number");
  b("earnings_th_mild_neg", -5, "stockAnalysis.configDescriptions.earningsThMildNeg", "number");
  b("earnings_th_strong_neg", -20, "stockAnalysis.configDescriptions.earningsThStrongNeg", "number");
  b("earnings_th_huge_neg", -50, "stockAnalysis.configDescriptions.earningsThHugeNeg", "number");
  // 质押风险分级阈值
  b("pledge_warning_line", 50, "stockAnalysis.configDescriptions.pledgeWarningLine", "number");
  b("pledge_liquidation_line", 70, "stockAnalysis.configDescriptions.pledgeLiquidationLine", "number");
  b("pledge_medium_line", 30, "stockAnalysis.configDescriptions.pledgeMediumLine", "number");
  b("pledge_low_line", 10, "stockAnalysis.configDescriptions.pledgeLowLine", "number");
  // 蒙特卡洛模拟默认参数
  b("mc_default_price", 10, "stockAnalysis.configDescriptions.mcDefaultPrice", "number");
  b("mc_default_return", 0.08, "stockAnalysis.configDescriptions.mcDefaultReturn", "number");
  b("mc_default_volatility", 0.3, "stockAnalysis.configDescriptions.mcDefaultVolatility", "number");
  b("mc_default_days", 30, "stockAnalysis.configDescriptions.mcDefaultDays", "number");
  b("mc_default_simulations", 1000, "stockAnalysis.configDescriptions.mcDefaultSimulations", "number");
  // 行业内估值/增长对比阈值
  b("industry_pe_cheap", 1.0, "stockAnalysis.configDescriptions.industryPeCheap", "number");
  b("industry_pe_expensive", 1.5, "stockAnalysis.configDescriptions.industryPeExpensive", "number");
  b("industry_growth_high", 1.2, "stockAnalysis.configDescriptions.industryGrowthHigh", "number");
  // 涨停潜力评分
  b("limit_pct_main", 10, "stockAnalysis.configDescriptions.limitPctMain", "number");
  b("limit_pct_star", 20, "stockAnalysis.configDescriptions.limitPctStar", "number");
  b("limit_pct_bj", 30, "stockAnalysis.configDescriptions.limitPctBj", "number");
  // 注意：vendor_* 9 个开关 + iwencai_key 不在这里暴露，
  // 由「数据源」tab（DataVendorsTab）全权管理，避免两边同时写造成竞态。
  // 全局开关
  b("analysis_dry_run", false, "stockAnalysis.configDescriptions.analysisDryRun", "boolean");
  return vars;
}
