// SPDX-License-Identifier: AGPL-3.0-only

//! portfolio 系列 Rhai 脚本依赖的 `pm_*` Rust 函数注册（函数体权威源）。
//!
//! ⚠ 本函数**不再被各入口直接调用**（2026-09-22 收敛）：唯一调用方是
//! `stock_workflow/rhai_registry.rs::register_axinvest_rhai_functions`
//! —— 那里是「AxInvest 需要哪些宿主函数」的唯一定义点，新增函数只改它。
//! 该单点入口再被两条路径消费：
//! 1. `init/services.rs::start_background_services` — 通过
//!    `register_shared_engine_initializer` 注入 rt-workflow 的
//!    `code_executor::shared_rhai_engine()`（DAG 主路径：portfolio-mgr /
//!    portfolio-risk-gate / data-quality 等 CodeNode 脚本）。
//! 2. `rhai_registry::build_stock_rhai_engine` — 供无法走共享 Engine 的入口
//!    （rerun 决策 `decision.rs`、What-If 回测 `commands/stock_analysis.rs`）
//!    构造独立 Engine。
//!
//! 历史 bug（2026-09-09 实证）：fork 清理「AxAgent 残留」时把
//! `register_portfolio_mgr_rhai_functions` 的调用整个移除，只留下 doc 注释，
//! 导致共享 Engine 上没有任何 `pm_*` 函数，data-quality 节点在
//! `pm_compute_factor_completeness` 处报 `Function not found` → VALIDATION_FAILED。
//!
//! 历史 bug 2（2026-09-22 实证）：恢复调用时写成**两处独立注册**（pm_* 与
//! bottleneck_* 各一次），而当时共享 Engine 的注册通道是单槽 `OnceLock` ⇒
//! 第二次被静默丢弃，`bottleneck_node_score` 从未生效。

use axagent_analysis_engine::portfolio_formula;
use rhai::Engine;

/// 把 portfolio 系列脚本依赖的全部 `pm_*` 函数注册到指定 Engine。
///
/// 纯数学/纯函数，无 DB / 无副作用；调用方负责沙箱限制（set_max_*）。
pub fn register_pm_functions(engine: &mut Engine) {
    // 裁定 P-B1（2026-10-17）：评分 → 因子信号的归一化口径**唯一权威在
    // `portfolio_formula::score_to_signal`**，脚本侧不再内联 `(x - 50.0) / 50.0`
    // （原先主链内联四处、逐档腿内联一处，五个 `50.0` 各写各的）。
    // ⚠ 必须**双载**：`totalScore`/`dqi_score` 来自 JSON 数值 ⇒ Rhai 侧是 INT，
    // 只注册 f64 会在运行时报 `Function not found`（拿不到分档、整段降级）。
    engine.register_fn("pm_score_to_signal", |score: i64| -> f64 {
        axagent_analysis_engine::portfolio_formula::score_to_signal(score as f64)
    });
    engine.register_fn("pm_score_to_signal", |score: f64| -> f64 {
        axagent_analysis_engine::portfolio_formula::score_to_signal(score)
    });
    engine.register_fn("pm_evidence_scale", |total_weight: f64, max_weight: f64| -> f64 {
        portfolio_formula::compute_evidence_scale(total_weight, max_weight)
    });
    engine.register_fn(
        "pm_kelly_position",
        |posterior: f64, odds: f64, cost_pct: f64, risk_level: &str| -> f64 {
            portfolio_formula::compute_kelly_position(posterior, odds, cost_pct, risk_level)
        },
    );
    // v156（PLAN-horizon-decision-repair.md §七 终审发现 ①）：风险档位仓位上限的**最终封顶**。
    // 四份逐档分支脚本在 `position = kelly × positionMultiplier` **之后**必须再过本函数 ——
    // 否则已按风险封顶的 kelly 会被档位乘数（长档 1.2）二次放大（极高 10 → 12 的口径穿透）。
    // 口径单一实现 = `portfolio_formula::apply_risk_cap`（与 `compute_kelly_position`
    // 内部用的是同一个函数 ⇒ 不存在第二份上限表）。
    engine.register_fn("pm_risk_cap", |position: f64, risk_level: &str| -> f64 {
        portfolio_formula::apply_risk_cap(position, risk_level)
    });
    // ⚠️⚠️ 死映射警示（2026-09-13 实证）：本函数注册进引擎，但**全仓零调用** ——
    //   仓库内所有 `*.rhai` 均无 `pm_classify_risk` 出现，DB `workflow_templates`
    //   全表扫描（nodes/tool_defs）同样为 0（正对照 `pm_evidence_scale` 命中 1 条，
    //   证明检索本身有效）。生产运行时的风险分类走的是 `portfolio-mgr.rhai` 内联的
    //   同构镜像（阈值取面板参数 `RISK_*`，本函数则是硬编码 V54 值）。
    //   ⇒ **改这里不会改变任何一次运行的结论**。要改风险判据，改 `portfolio-mgr.rhai`。
    //   保留原因：它是风险判据唯一的语义单测载体（analysis-engine::portfolio_formula）。
    //   待裁决：删除（去重）还是把 rhai 改调本函数（统一真相源，但会失去面板调参）。
    //   v78(2026-09-14)：第 7 参 `sector` 已删除（原用于「金融业白名单豁免」），与
    //   `portfolio-mgr.rhai` 对齐 —— 风险判据不再读取任何行业标签。
    engine.register_fn(
        "pm_classify_risk",
        |vol: rhai::Dynamic,
         sharpe: rhai::Dynamic,
         dd: rhai::Dynamic,
         roe: rhai::Dynamic,
         debt: rhai::Dynamic,
         growth: rhai::Dynamic|
         -> String {
            // P0 修复(2026-08-09): 原 6 个 Option<f64> 参数注册后不可调用（Rhai 1.25
            // 多 Option 参数闭包 Function not found），改为 Dynamic 参数内部转换。
            let f = |v: &rhai::Dynamic| -> Option<f64> {
                v.clone()
                    .try_cast::<f64>()
                    .or_else(|| v.clone().try_cast::<i64>().map(|x| x as f64))
            };
            portfolio_formula::classify_risk(
                f(&vol),
                f(&sharpe),
                f(&dd),
                f(&roe),
                f(&debt),
                f(&growth),
            )
        },
    );
    engine.register_fn("pm_risk_bias", |risk_level: &str| -> f64 {
        portfolio_formula::compute_risk_bias(risk_level)
    });
    engine.register_fn("pm_risk_veto", |action: &str, risk_level: &str| -> String {
        let (new_action, _, _) = portfolio_formula::apply_risk_veto(action, risk_level);
        new_action
    });
    // ── 决策否决门开关（2026-10-26 用户裁决）──────────────────────────────
    //
    // 三道否决门（风控 / 情绪 / 估值）+「顶层采纳档内买入」做成**运行期可切换**，
    // 这样产品决策（风控优先 vs 可执行性优先）**不必改图、不必升模板版本**。
    //
    // 门名用 `decision_gates::Gate::key()` 的**完整键**（如 `gate.risk_veto_enabled`），
    // 而**不是**短别名 —— 避免「同名不同义」：短别名（如 `risk`）在脚本与 DB 两处
    // 长得一样却可能指不同的门（MEMORY-RULES #738）。
    //
    // ⚠ 未知门名 ⇒ 返回 `false`（**不放行**）：脚本里若传错门名，
    // 保守方向必须是「仍按原门否决」而不是「静默放行」。
    engine.register_fn("pm_gate_enabled", |name: &str| -> bool {
        axagent_analysis_engine::decision_gates::is_open_by_name(name)
    });
    engine.register_fn(
        "pm_covariance_decay",
        |f1_w: f64, f3_w: f64, f9_w: f64, f11_w: f64, decay_target: &str| -> f64 {
            let (f9, f11) = portfolio_formula::apply_covariance_decay(f1_w, f3_w, f9_w, f11_w);
            match decay_target {
                "f9" => f9,
                "f11" => f11,
                _ => 0.0,
            }
        },
    );
    // P0: 贝叶斯因子置信度（基于 prior→posterior 的证据强度）
    engine.register_fn("pm_compute_bayes_confidence", |prior: f64, posterior: f64| -> f64 {
        portfolio_formula::compute_bayes_confidence(prior, posterior)
    });
    // 因子数据完整度：供 data-quality.rhai 评估因子层数据完整度
    // P0 修复(2026-08-09): Rhai 1.25 的 register_fn 对含多个 Option<T> 参数的闭包
    // 注册后无法调用（全 Some/全 None/混合均报 Function not found，已实测确认），
    // 改为 9 个 Dynamic 参数（万能类型，接受 f64/i64/&str/unit），闭包内转 Option。
    // 2026-09-12: 由 10 参降为 9 参 —— 移除 f7「trader_direction」（data-quality 是
    // trader 的上游，该因子恒缺失，详见 portfolio_formula::compute_factor_completeness 文档）。
    engine.register_fn(
        "pm_compute_factor_completeness",
        |total_score: rhai::Dynamic,
         consensus_score: rhai::Dynamic,
         catalyst_level: rhai::Dynamic,
         risk_volatility: rhai::Dynamic,
         valuation_dcf_upside: rhai::Dynamic,
         money_flow_main_net_inflow: rhai::Dynamic,
         lockup_shareholder_trades_len: rhai::Dynamic,
         announcements_len: rhai::Dynamic,
         pace_signal: rhai::Dynamic|
         -> f64 {
            // Rhai Dynamic 数值提取：f64/i64 均接受，unit/其他 → None
            let f = |v: &rhai::Dynamic| -> Option<f64> {
                v.clone()
                    .try_cast::<f64>()
                    .or_else(|| v.clone().try_cast::<i64>().map(|x| x as f64))
            };
            // into_string: ImmutableString/String → String，unit 报错 → None
            let s = |v: &rhai::Dynamic| v.clone().into_string().ok();
            let i = |v: &rhai::Dynamic| v.clone().try_cast::<i64>();
            portfolio_formula::compute_factor_completeness(
                f(&total_score),
                f(&consensus_score),
                s(&catalyst_level).as_deref(),
                f(&risk_volatility),
                f(&valuation_dcf_upside),
                f(&money_flow_main_net_inflow),
                i(&lockup_shareholder_trades_len),
                i(&announcements_len),
                f(&pace_signal),
            )
        },
    );
    // P1-E13: 组合风控门（portfolio-risk-gate.rhai 调用）。
    // 同样遵循 Dynamic 万能参数模式：target_price / stock_sector 在 Rhai 侧
    // 可能是 unit（safe_opt_* 兜底），闭包内转 Option。
    // v183: `target_price` 的**来源**换了（由 `portfolio-risk-gate.rhai` 用估值安全边际折算，
    // 不再由 input_mapping 注入 trader 报价），但参数个数必须与 rhai 调用、
    // `portfolio_formula::portfolio_risk_gate` 的签名三处同进退 —— 只改一处是
    // runtime「函数不存在」而不是编译错误。
    engine.register_fn(
        "pm_portfolio_risk_gate",
        |pm_action: &str,
         pm_position_pct: f64,
         pm_risk_level: &str,
         current_price: f64,
         target_price: rhai::Dynamic,
         stock_code: &str,
         stock_sector: rhai::Dynamic,
         holdings_json: &str,
         portfolio_cash: f64|
         -> String {
            let target = target_price
                .clone()
                .try_cast::<f64>()
                .or_else(|| target_price.clone().try_cast::<i64>().map(|x| x as f64));
            let sector =
                stock_sector.clone().try_cast::<rhai::ImmutableString>().map(|s| s.to_string());
            portfolio_formula::portfolio_risk_gate(
                pm_action,
                pm_position_pct,
                pm_risk_level,
                current_price,
                target,
                stock_code,
                sector.as_deref(),
                holdings_json,
                portfolio_cash,
            )
        },
    );
    // V66 修复(2026-07-29): 当前 portfolio-mgr.rhai 虽用本地词典未实际调用这两个函数，
    // 但保持注册对称可避免未来启用调用时 panic。
    engine.register_fn("pm_compute_news_sentiment", |title: &str, summary: &str| -> f64 {
        axagent_astock_data::sentiment::compute_news_sentiment(title, summary).unwrap_or(0.0)
    });
    engine.register_fn("pm_compute_text_sentiment", |text: &str| -> f64 {
        axagent_astock_data::sentiment::compute_text_sentiment(text).unwrap_or(0.0)
    });

    // S2(2026-09-26, PLAN-asof-replay-quality-attribution)：as-of 回放「设计性降级」
    // 的 vendor 方法清单（JSON 数组字符串），供 data-quality.rhai 豁免失败标记扣分。
    // live 模式（无 as-of 上下文）恒返回 "[]"。
    // R5 收紧(同日)：按「本轮运行基线之后的 seq」过滤（基线由 core.rs 运行入口写入），
    // 不按 as_of 日期过滤 —— 日期口径挡不住同截止日旧运行留在缓冲里的条目，
    // 实证（1ad42f59）会把全部 10 维无差别豁免、连真工具故障一起抹掉。
    engine.register_fn("pm_asof_degraded_methods", || -> String {
        if axagent_astock_data::as_of::current_as_of().is_none() {
            return "[]".to_string();
        }
        let methods = axagent_astock_data::as_of::global_degraded_methods_since_baseline();
        serde_json::to_string(&methods).unwrap_or_else(|_| "[]".to_string())
    });

    // B 批 2026-10-09：长线档 `positive_margin_of_safety` 加仓门的安全边际（**百分数**口径）。
    //
    // 为什么必须是宿主函数而不是节点 `input_mapping`：接线前脚本里写死 `30.0`，
    // 与 Rust 侧 `ideal_buy_price` 用的 0.30 是**同一把尺子的两份抄写**，而面板变量
    // `value_safety_margin`（默认 20）谁也不读。两条候选路线里：
    //   · `input_mapping` 要把键接进两处图（主模板 + 档子模板）并给「变量缺席」补一条
    //     `Variable not found` 的运行时风险，而换算（% ↔ 小数）会再多一个落点；
    //   · 宿主函数**原样转发** `mcp_tools::required_margin_of_safety_pct` ⇒ 两个载体
    //     读同一个函数、同一份快照，回落侧与量程判据只有一份。
    // 选后者。跨载体对账由 `check-panel-var-landing.mjs` 的 P3 钉住（Rust 侧有定义、
    // `.rhai` 侧有调用、且 `.rhai` 里不许再出现 `>= 30.0`）。
    //
    // ⚠ 命名带 `pm_` 前缀不是好看：`rhai_registry` 末尾的「脚本调用 ↔ 注册面」覆盖判据
    //   按 `pm_*` / `bottleneck_*` / `sim_*` 前缀识别宿主函数（无前缀的名字会被当成
    //   Rhai 内建而跳过检查，`band_for_score` 正是这样漏了半年）。带前缀 ⇒ 「漏注册」
    //   在 CI 就红，而不是运行期 `Function not found` 被脚本的 try/catch 吞成静默兜底。
    engine.register_fn(
        "pm_required_margin_pct",
        axagent_astock_data::mcp_tools::required_margin_of_safety_pct,
    );

    // ── 仿真验证（工作流 `sim-verify` 节点）─────────────────────────────────
    // 供 `sim-verify.rhai` 调用：在**决策之后**自动跑蒙特卡洛压力测试。
    //
    // 为什么必须走宿主函数、不能在脚本里算：共享 Rhai 引擎设了单次执行的
    // 操作数上限（`code_executor.rs` 的 `RHAI_MAX_OPERATIONS`），
    // 而仿真要遍历「场景 × 路径 × 事件」，写成脚本必然超限。宿主函数内部是
    // Rust 循环，对 Rhai 只计 1 次操作。
    //
    // 形参一律用 `rhai::Dynamic` 而非具体类型，两个原因都不能省：
    //   ① `input_mapping` 注入的数字被统一转成 f64（`code_executor.rs` 的
    //      `Value::Number` 分支），若形参写 `i64` 会 Function not found；
    //   ② Rhai 1.25 对多 `Option<T>` / 泛型参数注册有已知缺陷（见上文
    //      `pm_classify_risk` 的历史修复）。类型转换放在函数体内是唯一稳的形态。
    engine.register_fn(
        "sim_run_mc",
        |stock_code: rhai::Dynamic,
         reference_price: rhai::Dynamic,
         preset: rhai::Dynamic|
         -> String {
            let code = stock_code.clone().into_string().unwrap_or_default();
            let price = reference_price
                .clone()
                .try_cast::<f64>()
                .or_else(|| reference_price.clone().try_cast::<i64>().map(|v| v as f64))
                .unwrap_or(0.0);
            let preset = preset.clone().into_string().unwrap_or_else(|_| "stress".to_string());
            crate::market_sim_service::run_mc_preset(&code, price, &preset)
        },
    );
    // ── 已实现日波动率（四周期科学化 Phase D）──
    // 止损/止盈不再用「按日线经验拍的固定百分比」，而按 `k · σ_daily · √持有天数` 推导
    // （波动率目标法与三阶障碍法的共同基础：位移标准差 ∝ √时间）。
    // **口径实现只有一份**：`axagent_harness::indicators::realized_vol_pct`（样本标准差，
    // 与本文件其余 pm_* 一样在 Rust 侧实现并带单测）—— 不在 Rhai 里再抄一遍标准差，
    // 那正是「同一算法两处实现迟早漂移」的常见来源。
    // 返回 0.0 = **不可得**（样本不足 / 含非正价格 / 解析失败 / 持有 0 天）；脚本必须显式降级为
    // 固定百分比档并标 `stopSource = "fallback_pct"`，不得把 0 当成「零波动」算出 0 止损。
    engine.register_fn(
        "pm_vol_move_pct",
        |bars: rhai::Array, lookback: i64, holding_days: i64| -> f64 {
            let closes = closes_from_bars(&bars);
            axagent_harness::indicators::vol_move_pct(
                &closes,
                lookback.max(0) as usize,
                holding_days.max(0) as usize,
            )
            .unwrap_or(0.0)
        },
    );
    // P4′-b：波动带内的标准化位置（`(末价 − SMA) / σ_价`，Bollinger 式 z 值）。
    // 中档的 `inside_sigma_band` 门与长档的带内/带外判断都读它。
    // 口径只有一份（`harness::indicators::band_z`）—— 让四份分支脚本各自在 Rhai 里
    // 现算均值与标准差，就是给同一个统计量造四份实现，迟早漂移（禁区 12）。
    // 返回 unit = 窗口退化（停盘/一字板 σ=0）或样本不足 ⇒ 脚本必须点名「门未判定」，
    // 不得把「算不出」当成「正居中」。
    engine.register_fn("pm_band_z", |bars: rhai::Array, lookback: i64| -> rhai::Dynamic {
        let closes = closes_from_bars(&bars);
        match axagent_harness::indicators::band_z(&closes, lookback.max(0) as usize) {
            Some(z) => rhai::Dynamic::from(z),
            None => rhai::Dynamic::UNIT,
        }
    });
    // P0-5（`PLAN-stock-decision-reliability.md`）：近 5 日个股强势特征 —— 超短档
    // breadth 入场门在「全市场封板率 ≥ 0.5」之外再验**个股自身**（近 5 日出现过涨停，
    // 或区间累计涨幅达强势股量级）。封板率是市场级量（同一天所有票同一个数），没有个股
    // 前提时大盘情绪火热会给任何一只票放行，紫金矿业 2026-10-09 正是这样被推成「买入」。
    //
    // 返回 map `#{limitUpCount, cumGainPct, days}`；样本不足 / 无法解析 ⇒ unit
    // （脚本按「个股前提未判定 ≠ 通过」处理）。板块涨跌幅取 harness 权威表
    // （`detect_market_type` + `get_st_price_limit_pct`），本处不抄第二份。
    // ⚠ is_st 信息脚本侧没有 ⇒ 按非 ST 取板幅：ST 股 5cm 封板 (+4.8%) 不会被计为涨停，
    //   但累计涨幅通道仍可放行；ST 精确口径待个股微观结构（涨停池成员）接线时一并闭合。
    engine.register_fn(
        "pm_recent_strength",
        |bars: rhai::Array, stock_code: rhai::Dynamic, lookback: i64| -> rhai::Dynamic {
            let code = stock_code.into_string().unwrap_or_default();
            let market = axagent_harness::market_data::detect_market_type(&code);
            let limit_pct = axagent_harness::market_data::get_st_price_limit_pct(false, market);
            let closes = closes_from_bars(&bars);
            match axagent_harness::indicators::recent_strength(
                &closes,
                limit_pct,
                lookback.max(0) as usize,
            ) {
                Some(s) => {
                    let mut m = rhai::Map::new();
                    m.insert("limitUpCount".into(), rhai::Dynamic::from(s.limit_up_count as i64));
                    m.insert("cumGainPct".into(), rhai::Dynamic::from(s.cum_gain_pct));
                    m.insert("days".into(), rhai::Dynamic::from(s.days as i64));
                    rhai::Dynamic::from_map(m)
                },
                None => rhai::Dynamic::UNIT,
            }
        },
    );
    // P1-4（`PLAN-stock-decision-reliability.md` R6）：追涨形态的**客观量** ——
    // 当日涨幅 / 当日成交额 / 前窗（不含当日）成交额 90 分位 / 前高距离。
    // 门阈值（涨幅 4% / 90 分位 / 前高 5% 带）由决策脚本持有；样本不足 ⇒ unit
    // （门按「未判定 ≠ 通过」处理，与超短 breadth 门同纪律）。
    engine.register_fn("pm_surge_state", |bars: rhai::Array, lookback: i64| -> rhai::Dynamic {
        let (closes, amounts, highs) = surge_series_from_bars(&bars);
        match axagent_harness::indicators::surge_state(
            &closes,
            &amounts,
            &highs,
            lookback.max(0) as usize,
        ) {
            Some(s) => {
                let mut m = rhai::Map::new();
                m.insert("lastGainPct".into(), rhai::Dynamic::from(s.last_gain_pct));
                m.insert("lastAmount".into(), rhai::Dynamic::from(s.last_amount));
                m.insert(
                    "amountP90".into(),
                    match s.amount_p90 {
                        Some(v) => rhai::Dynamic::from(v),
                        None => rhai::Dynamic::UNIT,
                    },
                );
                m.insert(
                    "priorHigh".into(),
                    match s.prior_high {
                        Some(v) => rhai::Dynamic::from(v),
                        None => rhai::Dynamic::UNIT,
                    },
                );
                m.insert(
                    "highRatio".into(),
                    match s.high_ratio {
                        Some(v) => rhai::Dynamic::from(v),
                        None => rhai::Dynamic::UNIT,
                    },
                );
                rhai::Dynamic::from_map(m)
            },
            None => rhai::Dynamic::UNIT,
        }
    });
    // P1-8（`PLAN-stock-decision-reliability.md` R10）：宏观块时效门 ——
    // 返回**非空字符串** = 宏观块已超滞（调用方须把 macroRegime 腿按缺席处理并把原因进 gaps）。
    // 判据 = 发布日与分析锚点（snapshotDate）的**月差**：
    //   · GDP：> 3 个月（超 1 季度）⇒ stale；
    //   · CPI / PPI / PMI：> 1 个月 ⇒ stale；
    //   · 任一序列发布日缺失/不可解析 ⇒ 时效不可自证 ⇒ stale（不静默放行）。
    // 10-09 紫金实况（4419bd01）：GDP releaseDate 2026-04-20 距锚点 6 个月 ⇒ 命中。
    engine.register_fn(
        "pm_macro_stale_reason",
        |snapshot: rhai::Dynamic,
         gdp_release: rhai::Dynamic,
         cpi_release: rhai::Dynamic,
         ppi_release: rhai::Dynamic,
         pmi_release: rhai::Dynamic|
         -> rhai::Dynamic {
            let s = |d: rhai::Dynamic| d.into_string().unwrap_or_default();
            rhai::Dynamic::from(
                macro_stale_reason(
                    &s(snapshot),
                    &s(gdp_release),
                    &s(cpi_release),
                    &s(ppi_release),
                    &s(pmi_release),
                )
                .unwrap_or_default(),
            )
        },
    );
    // 「长线更值得」的判定侧折算（Phase D-2）：见 `harness::indicators::snr_confidence` 的推导。
    // 同样在 Rust 侧开方 —— 本仓共享 Rhai 引擎没有 sqrt/Math。
    engine.register_fn("pm_snr_confidence", |p: f64, holding_days: i64, anchor_days: i64| -> f64 {
        axagent_harness::indicators::snr_confidence(
            p,
            holding_days.max(0) as usize,
            anchor_days.max(0) as usize,
        )
    });
    // P5 仲裁：把「资金投向哪档」化为可比的**每日对数增长率**（Kelly 增长率 / 持有天数）。
    // 口径与理由只有一份，在 `portfolio_formula::kelly_growth_rate`（含单测）。
    // 返回 unit = 该档**不参与比较**（无仓位 / 赔率非正 / 持有 0 天 …）——
    // arbiter 必须把它当「退出」，不能当 0；当 0 就是「缺席 = 看空」（PLAN §十一 11-5）。
    engine.register_fn(
        "pm_kelly_growth",
        |posterior: f64, odds: f64, position_pct: f64, holding_days: i64| -> rhai::Dynamic {
            match axagent_analysis_engine::portfolio_formula::kelly_growth_rate(
                posterior,
                odds,
                position_pct,
                holding_days.max(0) as u32,
            ) {
                Some(g) => rhai::Dynamic::from(g),
                None => rhai::Dynamic::UNIT,
            }
        },
    );
    // ── P4′-b：逐档分支的**腿信号**（因子原始量 → [-1,1]）────────────────────
    // 单一实现在 `analysis-engine::leg_signal`，四份逐档决策脚本共用：同一因子的信号口径
    // 若按档各写一遍，就又回到「四份手抄阶梯迟早漂移」（AGENTS 禁区 12 / PLAN §二十四）。
    // 「档间不同」由 `horizon_branch_specs` 提供（腿集合 / 角色 / 配比 / 门 / 出场口径），
    // 不由这里各档一份实现提供。
    // 返回 Dynamic 而不是 Option<f64>：本仓共享引擎在 Rhai 1.25 上多 Option 参数/返回
    // 注册不稳（见上文 `pm_classify_risk` 的历史修复），缺席一律落成 unit，
    // 脚本用 `type_of(s) == "()"` 点名「该腿缺席」—— 缺席 ≠ 中性 0。
    engine.register_fn("pm_leg_signal", |factor: &str, inputs: rhai::Map| -> rhai::Dynamic {
        let packed = rhai::Dynamic::from(inputs.clone());
        let json =
            axagent_rt_workflow::work_engine::executors::data_transformer_executor::dynamic_to_json(
                packed,
            );
        match axagent_analysis_engine::leg_signal::leg_signal(factor, &json) {
            Some(v) => rhai::Dynamic::from(v),
            None => rhai::Dynamic::UNIT,
        }
    });
}

/// 从 K 线数组提取收盘价序列（兼容 `close` 为 f64 / i64 / 数字字符串三种上游形态）。
///
/// 解析不出来的 bar 直接跳过 —— 口径**比引擎侧 `num_of` 更宽**：`num_of`
/// （权威源 `crates/harness/src/rhai_engine.rs` 的 `register_common_functions`）只收 JSON Number
/// （f64 / 嵌层 i64），这里额外收「数字字符串」，因为工具节点的 K 线历史上就以字符串透出过 close。
/// 但**样本总数不足**时 `realized_vol_pct` 会返回 `None`（不是拿残缺序列硬算一个数）。
fn closes_from_bars(bars: &[rhai::Dynamic]) -> Vec<f64> {
    bars.iter()
        .filter_map(|b| {
            let map = b.clone().try_cast::<rhai::Map>()?;
            let close = map.get("close")?.clone();
            if let Some(v) = close.clone().try_cast::<f64>() {
                return Some(v);
            }
            if let Some(i) = close.clone().try_cast::<i64>() {
                return Some(i as f64);
            }
            close.into_immutable_string().ok().and_then(|s| s.trim().parse::<f64>().ok())
        })
        .collect()
}

/// K 线三元组（收盘 / 成交额 / 最高价）—— P1-4 追涨判据的输入，**逐行同进同出**
/// 保证下标对齐（`closes_from_bars` 是跳行式收集，单独配 amounts 会错位）。
///
/// 口径：只收「close 可解析」的行；`high` 缺失记 `NaN`（`surge_state` 侧跳过）；
/// 成交额按 `amount > 0 ? amount : volume × close`（本仓 K 线 amount 常为 0，
/// 与 `pm_avg_turnover` 的兜底同源），两源都缺记 0。
fn surge_series_from_bars(bars: &[rhai::Dynamic]) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    fn num(v: Option<&rhai::Dynamic>) -> Option<f64> {
        let v = v?;
        if let Some(f) = v.clone().try_cast::<f64>() {
            return Some(f);
        }
        if let Some(i) = v.clone().try_cast::<i64>() {
            return Some(i as f64);
        }
        v.clone().into_immutable_string().ok().and_then(|s| s.trim().parse::<f64>().ok())
    }
    let mut closes = Vec::with_capacity(bars.len());
    let mut amounts = Vec::with_capacity(bars.len());
    let mut highs = Vec::with_capacity(bars.len());
    for b in bars {
        let Some(map) = b.clone().try_cast::<rhai::Map>() else { continue };
        let Some(close) = num(map.get("close")) else { continue };
        let amount = num(map.get("amount"));
        let volume = num(map.get("volume"));
        let amt = match (amount, volume) {
            (Some(a), _) if a > 0.0 => a,
            (_, Some(v)) if v > 0.0 && close > 0.0 => close * v,
            _ => 0.0,
        };
        closes.push(close);
        amounts.push(amt);
        highs.push(num(map.get("high")).unwrap_or(f64::NAN));
    }
    (closes, amounts, highs)
}

/// 解析 `YYYY-MM-DD` 为 (年, 月)。空串 / 格式异常 ⇒ None。
/// P1-8 的宏观时效判据只做**月差**比较（不引入日历运算）。
fn parse_ym(s: &str) -> Option<(i64, u32)> {
    use chrono::Datelike;
    let d = chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok()?;
    Some((i64::from(d.year()), d.month()))
}

/// P1-8（`PLAN-stock-decision-reliability.md` R10）宏观时效判据（纯函数；注册闭包为薄包装）。
///
/// 返回 `Some(原因)` = 超滞或不可判定（调用方把 `macroRegime` 腿按缺席处理）；
/// `None` = 新鲜。判据 = 发布日与分析锚点（snapshotDate）的**月差**：
/// GDP > 3 个月、CPI/PPI/PMI > 1 个月 ⇒ 超滞；任一发布日缺失/不可解析 ⇒ 不可判定。
pub(crate) fn macro_stale_reason(
    snapshot: &str,
    gdp_release: &str,
    cpi_release: &str,
    ppi_release: &str,
    pmi_release: &str,
) -> Option<String> {
    let Some((anchor_y, anchor_m)) = parse_ym(snapshot) else {
        return Some("宏观时效不可判定：分析锚点日期缺失或格式异常".to_string());
    };
    let anchor_key = anchor_y * 12 + i64::from(anchor_m);
    let checks: [(&str, &str, i64); 4] = [
        ("GDP", gdp_release, 3),
        ("CPI", cpi_release, 1),
        ("PPI", ppi_release, 1),
        ("PMI", pmi_release, 1),
    ];
    for (name, rel, max_gap) in checks {
        let Some((rel_y, rel_m)) = parse_ym(rel) else {
            return Some(format!("{name} 发布日缺失或不可解析 ⇒ 宏观块时效不可自证，按未判定处理"));
        };
        let gap = anchor_key - (rel_y * 12 + i64::from(rel_m));
        if gap > max_gap {
            return Some(format!(
                "{name} 最新发布 {rel} 距分析锚点 {snapshot} 已 {gap} 个月（阈值 {max_gap}）⇒ 宏观块超滞，macroRegime 腿按缺席处理"
            ));
        }
    }
    None
}

#[cfg(test)]
mod macro_stale_reason_tests {
    use super::macro_stale_reason;

    #[test]
    fn fresh_block_passes_with_quarter_end_gdp() {
        // 齐备态（与 seed 一致性测试的夹具同形）：GDP 用 Q2 发布日（恰 3 个月，不超 3）、
        // 月度序列用最近一期发布。
        assert_eq!(
            macro_stale_reason(
                "2026-10-09",
                "2026-07-15",
                "2026-09-09",
                "2026-09-09",
                "2026-09-30"
            ),
            None
        );
    }

    #[test]
    fn stale_gdp_beyond_one_quarter_is_reported() {
        // 10-09 紫金实况（4419bd01）：GDP releaseDate 2026-04-20 ⇒ 距锚点 6 个月。
        let r = macro_stale_reason(
            "2026-10-09",
            "2026-04-20",
            "2026-09-09",
            "2026-09-09",
            "2026-09-30",
        )
        .expect("应判超滞");
        assert!(r.contains("GDP"), "原因须点名序列: {r}");
        assert!(r.contains("6 个月"), "原因须给出实测滞后: {r}");
    }

    #[test]
    fn stale_monthly_series_beyond_one_month_is_reported() {
        // CPI 卡在 7 月（release 8 月）⇒ 距 10 月锚点 2 个月 > 1。
        let r = macro_stale_reason(
            "2026-10-09",
            "2026-07-15",
            "2026-08-09",
            "2026-09-09",
            "2026-09-30",
        )
        .expect("应判超滞");
        assert!(r.contains("CPI"), "{r}");
    }

    #[test]
    fn missing_release_date_is_unjudged_not_fresh() {
        // 发布日缺失 ⇒ 不可自证 ⇒ 必须报出（不得静默放行）。
        assert!(
            macro_stale_reason("2026-10-09", "", "2026-09-09", "2026-09-09", "2026-09-30")
                .unwrap()
                .contains("GDP")
        );
        // 锚点缺失 ⇒ 全局不可判定。
        assert!(
            macro_stale_reason("", "2026-07-15", "2026-09-09", "2026-09-09", "2026-09-30")
                .unwrap()
                .contains("不可判定")
        );
    }

    #[test]
    fn future_release_date_does_not_trigger_stale() {
        // 非单调输入（脏数据）：release 晚于锚点 ⇒ gap 为负 ⇒ 不判超滞（不发明新语义）。
        assert_eq!(
            macro_stale_reason(
                "2026-10-09",
                "2026-11-01",
                "2026-09-09",
                "2026-09-09",
                "2026-09-30"
            ),
            None
        );
    }
}
