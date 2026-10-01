//! 股票分析工作流模板 — 可配置变量定义
//!
//! 拆分自 seed_stock_analysis.rs（P1-5），减少主文件行数 ~1100 行。
//! 变量通过 `{{var}}` 语法注入到 Rhai 脚本和 Agent prompt 中。

use axagent_harness::workflow_types::Variable;

/// `debate_rounds` 变量与建图展开轮数的默认值（单一权威源）。
///
/// 被三处引用保持一致，避免「同名散成多套值」：
///   1. 下方 `debate_rounds` 变量的 `value`（最终落库的变量表）；
///   2. `seed_stock_analysis.rs` 借 `resolve_debate_rounds` 从旧变量解析建图轮数。
pub(crate) const DEFAULT_DEBATE_ROUNDS: u32 = 3;

/// `kline_limit` 的默认值（**单一权威源**）—— 分析师链的 K 线取数根数。
///
/// ## 为什么名字带 `ANALYST`（而不是复用已有的 `DEFAULT_KLINE_LIMIT`）
///
/// 仓内已有一个**同名**常量 `axagent_quant::kline_provider::DEFAULT_KLINE_LIMIT = 504`
/// （回测取数，`~2 年日线`；前端 `quant/tabs/WfDesTab.tsx` 另有本地的 600）。
/// 那不是同一个量：**回测要的是「够切 5+ fold」，分析师要的是「够算 250 日均线」**，
/// 同值复用会把回测的 fold 需求塞进一次分析请求（504 根 ≈ 2 倍提示词体积）。
/// 按 `AGENTS.md` 禁区 12「禁止重复定义」，做法**不是**新造一个同名的 250（那才是重定义），
/// 而是**换名消歧**并在此写明三者不同源 —— 三处各自的消费语义见本条与那句注释。
///
/// ## 为什么是 250（而不是历史值 120）
///
/// `agency_experts/stock-analysis/market-analyst.md` 的方法论第 1 条明确要求：
/// 「读 K 线数据（**30/60/120/250 日均线**状态、近期高低点、成交量变化）」——
/// 即提示词的**硬需求**是 250 根日线。而 `get_stock_kline` 的 `limit` 缺省值恰为
/// **120**（`crates/astock-data/src/mcp_tools.rs` 的 `unwrap_or(120).min(500)`），
/// 且本变量自建立以来**从未接进任何 tool 节点**（全仓 grep：只出现在种子定义、设置面板
/// 与单测里，`seed_stock_analysis.rs` 一次都没引用）⇒ 实际取到的恒是 120 根。
///
/// 实证（2026-10-01 运行 `a7e590a4`，600406 国电南瑞）：`t-market-data` 的
/// `result.content` 是 **120 根**、首根 2026-04-09，而技术面分析师报告原文写着
/// 「**250日均线数据缺失**（数据仅覆盖4月至今）」——该句命中失败标记词表、
/// 把技术面节点压进「⚠️ 低置信」。这不是分析师措辞问题，是**提示词要的数据物理上没给**：
/// 120 根算得出 MA120，**算不出 MA250**（且该形态每轮每只票都存在，只是分析师未必写出来）。
///
/// 取 250 而非更大：MA250 = 最近 250 根收盘的均值，250 根**恰好**够；
/// 工具上限 500，留档位给用户在面板上调深。
pub(crate) const DEFAULT_ANALYST_KLINE_LIMIT: u32 = 250;

// 编译期断言：本值必须够算 250 日均线（提示词 `market-analyst.md` 的硬需求）。
// 为什么是**编译期**而不是测试：这是「常量 vs 需求」的跨文件约束，
// 测试要靠人记得跑，而它的失效形态恰恰是「有人为了省 token 把它调小、没人注意到」
// （同 `seed_stock_analysis.rs` 里 DCF/K 线两个迁移门断言的安置理由）。
// 用 `const _: () = assert!(…)` 形态：仓内既有先例，clippy 不报 `assertions_on_constants`
// （该 lint 只作用于运行期断言 —— 首个版本写在测试里，`-D warnings` 当场红）。
const _: () = assert!(
    DEFAULT_ANALYST_KLINE_LIMIT >= 250,
    "kline_limit 默认值不足以计算 250 日均线（须 ≥ 250）：market-analyst.md 第 1 条要求「30/60/120/250 日均线状态」"
);

/// DCF 估值参数的默认值（**单一权威源**，单位 = 百分数）。
///
/// 被两处引用，避免同名散成多套值：
///   1. 下方 `value_dcf_growth_rate` / `value_dcf_perpetual_rate` /
///      `value_dcf_discount_rate` 三个变量的 `value`（种子默认值）；
///   2. `seed_stock_analysis.rs` 的 **v74 强制覆写**（`force_variable_value`）。
///
/// ✅ **同源已由代码保证**（2026-09-22 起）：下方三个常量**派生自**
/// `astock-data::mcp_tools` 的 `DEFAULT_GROWTH` / `PERPETUAL_GROWTH` / `DISCOUNT_RATE`
/// （×100 换算为百分数），**不再是手抄字面量** —— 此前该处手抄的值与 astock-data
/// 常量曾各停在不同校准批次上（且本处恰好是"校准后"、astock-data 是"校准前"的反例）。
///
/// 业务理由**不在本文件**，也不在 `analysis-engine::decision::ValueConfig`
/// （该结构 2026-09-23 起本身也已**派生**自 `astock-data`，见其 `default_dcf_*` 注释；
/// 它当前全仓无消费方，且历史上正是「假修复」的载体）—— 唯一的依据声明在
/// `astock_data::mcp_tools::{PERPETUAL_GROWTH, DISCOUNT_RATE, RISK_FREE_RATE}` 的文档注释。
/// 引用方向必须是「本文件 → astock-data」，**不得**反向引用 `decision::ValueConfig`：
/// 后者是派生端，被当依据会导致下一次校准又只改到它、改不到实际执行的常量。
///
/// ## 为什么需要第 2 处引用（v74 的由来）
///
/// 这组值 2026-09-12 就校准过（原 10% / 3% / 8% → 8.5% / 4% / 12%），
/// 但**从未在生产生效**：`merge_variable_values` 的语义是「新定义 + 无条件保留
/// 旧值」，DB 存量一直是 10 / 3 / 8 —— 改代码常量与种子默认值都是**假修复**。
/// 实测（2026-09-21，688114 华大智造）：三值恰为校准前的原值，致 `dcf.mid`
/// 24.23（校正后 37.27，**低估 35%**），且三个方向**一致压低**成长股估值。
/// ⇒ 只改默认值不够，必须经 `force_variable_value` 覆写存量。
pub(crate) const DEFAULT_DCF_GROWTH_RATE_PCT: f64 =
    axagent_astock_data::mcp_tools::DEFAULT_GROWTH * 100.0;
pub(crate) const DEFAULT_DCF_PERPETUAL_RATE_PCT: f64 =
    axagent_astock_data::mcp_tools::PERPETUAL_GROWTH * 100.0;
pub(crate) const DEFAULT_DCF_DISCOUNT_RATE_PCT: f64 =
    axagent_astock_data::mcp_tools::DISCOUNT_RATE * 100.0;

/// 构建股票分析工作流模板的所有可配置变量
pub(crate) fn build_template_variables() -> Vec<Variable> {
    vec![
        // ── 分析流程参数 ──
        Variable {
            name: "analysis_depth".into(),
            var_type: "enum".into(),
            value: serde_json::json!("standard"),
            description: Some("分析深度: quick / standard / deep".into()),
            is_secret: false,
        },
        Variable {
            name: "debate_rounds".into(),
            var_type: "number".into(),
            // 多空辩论轮数。每轮展开一对独立辩手节点（bull-rN/bear-rN），经各自的
            // expert persona 与 context_sources（引用前序轮次输出）产出不同内容——
            // 即「真多轮」，每轮各自真跑一次 LLM。
            // ⚠️ 该值在建图时由 `seed_stock_analysis.rs` 从本变量读取并展开节点，
            //   改动后需升 TEMPLATE_VERSION 强制重种才会生效（版本门见 seed）。
            value: serde_json::json!(DEFAULT_DEBATE_ROUNDS),
            description: Some(
                "多空辩论轮数（1~3）。每轮展开一对独立辩手节点，各轮经不同专家 \
                 persona 与前序辩论输出承接，逐轮真跑。建议 ≤3 以控制链尾请求体积。"
                    .into(),
            ),
            is_secret: false,
        },
        Variable {
            name: "screening_source".into(),
            var_type: "string".into(),
            value: serde_json::json!(""),
            description: Some("筛选来源标记：serenity(瓶颈掘金) / ''(直接分析)".into()),
            is_secret: false,
        },
        Variable {
            name: "max_concurrent".into(),
            var_type: "number".into(),
            // 2026-09-08: 3→8。DB 存量 v7 实际值仍为 3（旧种子遗留），3 个并发槽被
            // 429 重试节点占住不放时其余分析师排队等待，事实串行化（PG 时间线实证）。
            value: serde_json::json!(8),
            description: Some("并行分析的 Agent 数量上限".into()),
            is_secret: false,
        },
        // ── 数据源参数 ──
        Variable {
            name: "kline_period".into(),
            var_type: "enum".into(),
            value: serde_json::json!("daily"),
            description: Some("K线周期: daily / weekly / monthly".into()),
            is_secret: false,
        },
        Variable {
            name: "kline_limit".into(),
            var_type: "number".into(),
            // ⚠ 「值」与「有消费方」是两件事：本变量长期只是**声明**（默认 120、面板可调、
            //   全仓无人引用），v115 才由 `t-market-data` 节点真正接上
            //   （`seed_stock_analysis.rs`：`tool_node(…, &[("limit", "kline_limit")], …)`）。
            //   默认值取 250 的理由见 `DEFAULT_ANALYST_KLINE_LIMIT` 的文档注释。
            value: serde_json::json!(DEFAULT_ANALYST_KLINE_LIMIT),
            description: Some("K线获取根数 (1-500)；≥250 才够 30/60/120/250 日均线口径".into()),
            is_secret: false,
        },
        Variable {
            name: "news_limit".into(),
            var_type: "number".into(),
            value: serde_json::json!(30),
            description: Some("新闻获取条数 (1-100)".into()),
            is_secret: false,
        },
        // ── 行业参照参数（股票分析中 t-baseline-* 节点使用）──
        Variable {
            name: "ref_semi_code".into(),
            var_type: "string".into(),
            value: serde_json::json!("002371"),
            description: Some("半导体行业参照股票代码".into()),
            is_secret: false,
        },
        Variable {
            name: "ref_battery_code".into(),
            var_type: "string".into(),
            value: serde_json::json!("300750"),
            description: Some("电池行业参照股票代码".into()),
            is_secret: false,
        },
        Variable {
            name: "ref_chem_code".into(),
            var_type: "string".into(),
            value: serde_json::json!("600309"),
            description: Some("化工行业参照股票代码".into()),
            is_secret: false,
        },
        Variable {
            name: "ref_med_code".into(),
            var_type: "string".into(),
            value: serde_json::json!("688981"),
            description: Some("医疗行业参照股票代码".into()),
            is_secret: false,
        },
        Variable {
            name: "ref_aero_code".into(),
            var_type: "string".into(),
            value: serde_json::json!("600760"),
            description: Some("航空军工行业参照股票代码".into()),
            is_secret: false,
        },
        Variable {
            name: "ref_consumer_elec_code".into(),
            var_type: "string".into(),
            value: serde_json::json!("002475"),
            description: Some("消费电子行业参照股票代码".into()),
            is_secret: false,
        },
        Variable {
            name: "ref_auto_code".into(),
            var_type: "string".into(),
            value: serde_json::json!("600104"),
            description: Some("汽车行业参照股票代码".into()),
            is_secret: false,
        },
        // ── Agent 节点 LLM 参数 ──
        Variable {
            name: "agent_temperature".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.3),
            description: Some("所有 Agent 节点 LLM 温度 (0-2)".into()),
            is_secret: false,
        },
        Variable {
            name: "agent_max_tokens".into(),
            var_type: "number".into(),
            value: serde_json::json!(32768),
            description: Some(
                "所有 Agent 节点最大输出 token 数（模板变量覆盖硬编码默认值）".into(),
            ),
            is_secret: false,
        },
        Variable {
            name: "agent_timeout_secs".into(),
            var_type: "number".into(),
            value: serde_json::json!(600),
            description: Some(
                "每个 Agent 节点执行超时秒数（主控，继承到所有无显式超时的 agent 节点）".into(),
            ),
            is_secret: false,
        },
        Variable {
            name: "agent_retry_max".into(),
            var_type: "number".into(),
            value: serde_json::json!(2),
            description: Some("每个 Agent 节点最大重试次数".into()),
            is_secret: false,
        },
        // ── Tool 节点参数 ──
        Variable {
            name: "tool_timeout_secs".into(),
            var_type: "number".into(),
            value: serde_json::json!(30),
            description: Some("每个 Tool 节点执行超时秒数".into()),
            is_secret: false,
        },
        Variable {
            name: "tool_retry_max".into(),
            var_type: "number".into(),
            value: serde_json::json!(2),
            description: Some("每个 Tool 节点最大重试次数".into()),
            is_secret: false,
        },
        // ── 评分权重 ──
        Variable {
            name: "scoring_trend".into(),
            var_type: "number".into(),
            value: serde_json::json!(30.0),
            description: Some("趋势评分权重 (0-100)".into()),
            is_secret: false,
        },
        Variable {
            name: "scoring_deviation".into(),
            var_type: "number".into(),
            value: serde_json::json!(20.0),
            description: Some("偏离度评分权重 (0-100)".into()),
            is_secret: false,
        },
        Variable {
            name: "scoring_macd".into(),
            var_type: "number".into(),
            value: serde_json::json!(15.0),
            description: Some("MACD 评分权重 (0-100)".into()),
            is_secret: false,
        },
        Variable {
            name: "scoring_volume".into(),
            var_type: "number".into(),
            value: serde_json::json!(15.0),
            description: Some("成交量评分权重 (0-100)".into()),
            is_secret: false,
        },
        Variable {
            name: "scoring_rsi".into(),
            var_type: "number".into(),
            value: serde_json::json!(10.0),
            description: Some("RSI 评分权重 (0-100)".into()),
            is_secret: false,
        },
        Variable {
            name: "scoring_support".into(),
            var_type: "number".into(),
            value: serde_json::json!(10.0),
            description: Some("支撑阻力评分权重 (0-100)".into()),
            is_secret: false,
        },
        Variable {
            name: "scoring_boll".into(),
            var_type: "number".into(),
            value: serde_json::json!(5.0),
            description: Some("布林带评分权重 (0-100)".into()),
            is_secret: false,
        },
        // ── 规则引擎阈值 ──
        Variable {
            name: "rule_rsi_overbought".into(),
            var_type: "number".into(),
            value: serde_json::json!(80.0),
            description: Some("RSI 超买阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "rule_rsi_oversold".into(),
            var_type: "number".into(),
            value: serde_json::json!(20.0),
            description: Some("RSI 超卖阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "rule_bias_limit_pct".into(),
            var_type: "number".into(),
            value: serde_json::json!(5.0),
            description: Some("均线偏离极限 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "rule_volume_signal_block".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            description: Some("成交量异常时是否阻塞信号".into()),
            is_secret: false,
        },
        Variable {
            name: "rule_bear_low_score".into(),
            var_type: "number".into(),
            value: serde_json::json!(30),
            description: Some("空方低分阈值 (低于此分数触发警告)".into()),
            is_secret: false,
        },
        Variable {
            name: "rule_auto_stop_loss_pct".into(),
            var_type: "number".into(),
            value: serde_json::json!(5.0),
            description: Some("自动止损线 (%)".into()),
            is_secret: false,
        },
        // ── 仓位限制 ──
        Variable {
            name: "pos_max_single_pct".into(),
            var_type: "number".into(),
            value: serde_json::json!(20.0),
            description: Some("单只股票最大仓位占比 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "pos_max_total".into(),
            var_type: "number".into(),
            value: serde_json::json!(10),
            description: Some("最大持仓数量".into()),
            is_secret: false,
        },
        Variable {
            name: "pos_max_sector_pct".into(),
            var_type: "number".into(),
            value: serde_json::json!(40.0),
            description: Some("最大行业暴露占比 (%)".into()),
            is_secret: false,
        },
        // ── 估值参数（A股校准：唯一依据声明见 astock-data::mcp_tools 的常量文档）──
        Variable {
            name: "value_dcf_growth_rate".into(),
            var_type: "number".into(),
            value: serde_json::json!(DEFAULT_DCF_GROWTH_RATE_PCT),
            description: Some("DCF 增长率 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "value_dcf_perpetual_rate".into(),
            var_type: "number".into(),
            value: serde_json::json!(DEFAULT_DCF_PERPETUAL_RATE_PCT),
            description: Some("DCF 永续增长率 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "value_dcf_discount_rate".into(),
            var_type: "number".into(),
            value: serde_json::json!(DEFAULT_DCF_DISCOUNT_RATE_PCT),
            description: Some("DCF 折现率 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "value_moat_threshold".into(),
            var_type: "number".into(),
            value: serde_json::json!(60),
            description: Some("护城河评分阈值 (0-100)".into()),
            is_secret: false,
        },
        Variable {
            name: "value_fscore_buy".into(),
            var_type: "number".into(),
            value: serde_json::json!(7),
            description: Some("F-Score 买入阈值 (0-9)".into()),
            is_secret: false,
        },
        Variable {
            name: "value_safety_margin".into(),
            var_type: "number".into(),
            value: serde_json::json!(20.0),
            description: Some("安全边际最低折扣 (%)".into()),
            is_secret: false,
        },
        // ── 监控参数 ──
        Variable {
            name: "monitor_poll_interval_secs".into(),
            var_type: "number".into(),
            value: serde_json::json!(30),
            description: Some("监控轮询间隔秒数".into()),
            is_secret: false,
        },
        Variable {
            name: "monitor_change_pct".into(),
            var_type: "number".into(),
            value: serde_json::json!(5.0),
            description: Some("价格异动提醒阈值 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "monitor_turnover".into(),
            var_type: "number".into(),
            value: serde_json::json!(10.0),
            description: Some("换手率异动提醒阈值 (%)".into()),
            is_secret: false,
        },
        // ── 置信度参数 ──
        Variable {
            name: "min_confidence".into(),
            var_type: "number".into(),
            value: serde_json::json!(60),
            description: Some("最低置信度阈值 (低于此值建议观望)".into()),
            is_secret: false,
        },
        // ── 数据源供应商开关 ──
        Variable {
            name: "vendor_tencent".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            description: Some("腾讯财经 — 报价数据".into()),
            is_secret: false,
        },
        Variable {
            name: "vendor_eastmoney".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            description: Some("东方财富 — 财务/K线数据".into()),
            is_secret: false,
        },
        Variable {
            name: "vendor_sina".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            // 描述必须点名资金流：本机 push2his 连接级拒绝，sina 是回放里
            // **唯一**可用的按日资金流通道（as-of T17）——关掉它等于资金流维度必降级。
            description: Some(
                "新浪财经 — 新闻数据 / 资金流按日历史（回放里唯一的资金流通道）".into(),
            ),
            is_secret: false,
        },
        Variable {
            name: "vendor_ths".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            description: Some("同花顺 — 综合数据".into()),
            is_secret: false,
        },
        Variable {
            name: "vendor_cninfo".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            description: Some("巨潮资讯 — 信息披露".into()),
            is_secret: false,
        },
        Variable {
            name: "vendor_baidu_stock".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            description: Some("百度股票 — 数据".into()),
            is_secret: false,
        },
        Variable {
            name: "vendor_iwencai".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            description: Some("问财 — 选股数据".into()),
            is_secret: false,
        },
        Variable {
            name: "vendor_akshare".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            description: Some("AKShare — 开源数据".into()),
            is_secret: false,
        },
        Variable {
            name: "vendor_mootdx".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            description: Some("Mootdx — 本地行情接口".into()),
            is_secret: false,
        },
        // ── 需要 Token/Key 的数据源（开关默认关闭，需用户手动配置凭据后开启）──
        Variable {
            name: "vendor_xueqiu".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(false),
            description: Some("雪球 — 需配置 xq_a_token".into()),
            is_secret: false,
        },
        Variable {
            name: "vendor_neodata".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(false),
            description: Some("NeoData — 需配置 API Token".into()),
            is_secret: false,
        },
        // ── 凭据变量（is_secret=true，前端设置页管理）──
        // 后端 stock_analysis.rs / core.rs 通过变量名读取并注入到 vendor
        Variable {
            name: "vendor_iwencai_key".into(),
            var_type: "string".into(),
            value: serde_json::json!(""),
            description: Some("问财 API Key".into()),
            is_secret: true,
        },
        Variable {
            name: "vendor_xueqiu_token".into(),
            var_type: "string".into(),
            value: serde_json::json!(""),
            description: Some("雪球 xq_a_token".into()),
            is_secret: true,
        },
        Variable {
            name: "vendor_neodata_token".into(),
            var_type: "string".into(),
            value: serde_json::json!(""),
            description: Some("NeoData API Token".into()),
            is_secret: true,
        },
        // ── 金融模型参数 ──
        Variable {
            name: "risk_free_rate".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.03),
            description: Some("无风险利率".into()),
            is_secret: false,
        },
        Variable {
            name: "var_confidence".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.95),
            description: Some("VaR 置信度 (0-1)".into()),
            is_secret: false,
        },
        Variable {
            name: "outlier_method".into(),
            var_type: "enum".into(),
            value: serde_json::json!("zscore"),
            description: Some("异常值检测方法: zscore / iqr".into()),
            is_secret: false,
        },
        Variable {
            name: "outlier_threshold".into(),
            var_type: "number".into(),
            value: serde_json::json!(2.0),
            description: Some("异常值 Z-score 阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "kelly_fraction".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.5),
            description: Some("凯利仓位系数".into()),
            is_secret: false,
        },
        Variable {
            name: "kelly_min_win_rate".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.4),
            description: Some("凯利最低胜率要求 (0-1)".into()),
            is_secret: false,
        },
        Variable {
            name: "kelly_min_odds".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.0),
            description: Some("凯利最低赔率要求".into()),
            is_secret: false,
        },
        Variable {
            name: "cost_pct".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.003),
            description: Some("交易成本率 (0.003=0.3%)".into()),
            is_secret: false,
        },
        // ── 金融模型补充参数 ──
        Variable {
            name: "risk_sharpe_annualization".into(),
            var_type: "number".into(),
            value: serde_json::json!(252),
            description: Some("夏普比率年化因子（交易日数）".into()),
            is_secret: false,
        },
        Variable {
            name: "risk_kelly_heavy_threshold".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.25),
            description: Some("凯利重度仓位阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "risk_kelly_medium_threshold".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.1),
            description: Some("凯利中度仓位阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "kelly_default_win_rate".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.5),
            description: Some("凯利默认胜率 (0-1)".into()),
            is_secret: false,
        },
        Variable {
            name: "kelly_default_avg_win".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.05),
            description: Some("凯利默认平均盈利率".into()),
            is_secret: false,
        },
        Variable {
            name: "kelly_default_avg_loss".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.05),
            description: Some("凯利默认平均亏损率".into()),
            is_secret: false,
        },
        // ── 组合风控 ──
        Variable {
            name: "risk_max_drawdown_limit".into(),
            var_type: "number".into(),
            value: serde_json::json!(15.0),
            description: Some("组合最大回撤熔断线 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "risk_max_daily_loss_pct".into(),
            var_type: "number".into(),
            value: serde_json::json!(3.0),
            description: Some("单日最大亏损 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "risk_correlation_lookback_days".into(),
            var_type: "number".into(),
            value: serde_json::json!(60),
            description: Some("相关性回看天数".into()),
            is_secret: false,
        },
        // ── 信号检测参数 ──
        Variable {
            name: "signal_rsi_oversold".into(),
            var_type: "number".into(),
            value: serde_json::json!(30.0),
            description: Some("RSI 超卖信号阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "signal_rsi_overbought".into(),
            var_type: "number".into(),
            value: serde_json::json!(70.0),
            description: Some("RSI 超买信号阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "signal_ma_fast".into(),
            var_type: "number".into(),
            value: serde_json::json!(5),
            description: Some("MA 金叉检测快线周期".into()),
            is_secret: false,
        },
        Variable {
            name: "signal_ma_slow".into(),
            var_type: "number".into(),
            value: serde_json::json!(20),
            description: Some("MA 金叉检测慢线周期".into()),
            is_secret: false,
        },
        Variable {
            name: "signal_breakout_volume_mult".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.5),
            description: Some("突破/破位放量倍数阈值".into()),
            is_secret: false,
        },
        // ── 关键价位参数 ──
        Variable {
            name: "keylevel_lookback_days".into(),
            var_type: "number".into(),
            value: serde_json::json!(60),
            description: Some("关键价位回看窗口 (交易日)".into()),
            is_secret: false,
        },
        Variable {
            name: "keylevel_touch_tolerance_pct".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.0),
            description: Some("关键价位触碰容差 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "keylevel_min_touches".into(),
            var_type: "number".into(),
            value: serde_json::json!(2),
            description: Some("确认支撑/阻力最少触碰次数".into()),
            is_secret: false,
        },
        // ── 监控告警参数 ──
        Variable {
            name: "monitor_alert_cooldown_secs".into(),
            var_type: "number".into(),
            value: serde_json::json!(300),
            description: Some("同一标的告警冷却时间 (秒)".into()),
            is_secret: false,
        },
        Variable {
            name: "monitor_min_severity".into(),
            var_type: "enum".into(),
            value: serde_json::json!("info"),
            description: Some("最低推送告警等级: info / warn / critical".into()),
            is_secret: false,
        },
        Variable {
            name: "monitor_channels".into(),
            var_type: "string".into(),
            value: serde_json::json!("in_app"),
            description: Some("推送渠道，逗号分隔: in_app / lark / email / webhook".into()),
            is_secret: false,
        },
        // ── 推荐器策略开关 ──
        Variable {
            name: "reco_trend_enabled".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            description: Some("启用趋势跟踪子策略".into()),
            is_secret: false,
        },
        Variable {
            name: "reco_reversion_enabled".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            description: Some("启用超跌反弹子策略".into()),
            is_secret: false,
        },
        Variable {
            name: "reco_value_enabled".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            description: Some("启用价值选股子策略".into()),
            is_secret: false,
        },
        Variable {
            name: "reco_capital_enabled".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            description: Some("启用资金流向子策略".into()),
            is_secret: false,
        },
        Variable {
            name: "reco_watchlist_enabled".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            description: Some("启用自选股策略".into()),
            is_secret: false,
        },
        Variable {
            name: "reco_min_confidence".into(),
            var_type: "number".into(),
            // 出厂 60 与各策略的置信度尺度不匹配：capital≈71 / trend≈70-75 /
            // reversion≈69 / value≈67，而 watchlist≈50；且超短线周期会额外乘
            // 反身性折扣 ×0.85（recommender/mod.rs），使超短线全体系统性降 15%。
            // 60 的门槛下 value(57)/reversion(59)/watchlist(43) 的超短线产出被
            // 全量滤除 → 面板只剩 synthetic 占位。50 让全部主策略可通过。
            value: serde_json::json!(50),
            description: Some(
                "推荐器最低置信度 (0-100)。各策略置信度尺度不同，且超短线周期额外 ×0.85 折扣；\
                 设为 ≥60 会滤掉 value/reversion/watchlist 的超短线产出"
                    .into(),
            ),
            is_secret: false,
        },
        Variable {
            name: "reco_conf_sensitivity".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.0),
            description: Some("评分进入逐档先验 logit 合成的斜率 s（0=只认先验，越大越信评分；Phase R-C）".into()),
            is_secret: false,
        },
        Variable {
            name: "reco_ic_gate".into(),
            var_type: "string".into(),
            // Q2 裁定「shadow 起步」：闭环照算、照留痕、面板可读，但不覆盖评分消费的权重；
            // as-of 回放 A/B 过判据后才人工转 on。读不到本变量时消费侧同样按 shadow 处理。
            value: serde_json::json!("shadow"),
            description: Some(
                "荐股反思闭环生效闸：off=停用；shadow=只计算不参与评分；on=用 reco-loop 逐格权重\
                 覆盖静态 reco_strategy_weights（样本不足/IC 不可测的格自动回基线 1.0，只降不升；\
                 PLAN-reco-reflection-closure.md）"
                    .into(),
            ),
            is_secret: false,
        },
        // 窗口涨幅达标漏检核查的四档阈值（`PLAN-mover-recall-attribution.md`）。
        // 出厂值来自用户裁定：超短 10%、短 20%、中 30%、长 40%。
        // 口径边界：**绝对涨幅，不含板块涨停语义**，故变量名与文案一律不得写「涨停」；
        // 窗口天数不在这里（唯一来源是 `harness::holding_period::default_holding_days`）。
        Variable {
            name: "mover_gain_ultra_short".into(),
            var_type: "number".into(),
            value: serde_json::json!(10.0),
            description: Some("超短档窗口累计涨幅达标阈值（%）".into()),
            is_secret: false,
        },
        Variable {
            name: "mover_gain_short".into(),
            var_type: "number".into(),
            value: serde_json::json!(20.0),
            description: Some("短档窗口累计涨幅达标阈值（%）".into()),
            is_secret: false,
        },
        Variable {
            name: "mover_gain_mid".into(),
            var_type: "number".into(),
            value: serde_json::json!(30.0),
            description: Some("中档窗口累计涨幅达标阈值（%）".into()),
            is_secret: false,
        },
        Variable {
            name: "mover_gain_long".into(),
            var_type: "number".into(),
            value: serde_json::json!(40.0),
            description: Some("长档窗口累计涨幅达标阈值（%）".into()),
            is_secret: false,
        },
        Variable {
            name: "reco_stop_vol_mult".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.2),
            description: Some("荐股止损倍数 k1：止损距离 = k1 × 日线σ × √持有天数（Phase R-D）".into()),
            is_secret: false,
        },
        Variable {
            name: "reco_target_vol_mult".into(),
            var_type: "number".into(),
            value: serde_json::json!(2.0),
            description: Some("荐股止盈倍数 k2：目标位移 = k2 × 日线σ × √持有天数".into()),
            is_secret: false,
        },
        Variable {
            name: "reco_risk_budget_pct".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.5),
            description: Some("单票风险预算 R（%）：仓位 = min(策略上限, 100×R/止损%) ⇒ 止损被打掉时组合恰损失 R%".into()),
            is_secret: false,
        },
        Variable {
            name: "reco_round_trip_cost_pct".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.6),
            description: Some("往返换手成本（%）：按该档目标位移摊薄成成本拖累，短档自动多扣".into()),
            is_secret: false,
        },
        // ── 决策回溯参数 ──
        Variable {
            name: "decision_max_history_per_stock".into(),
            var_type: "number".into(),
            value: serde_json::json!(50),
            description: Some("每只股票保留的历史决策条数".into()),
            is_secret: false,
        },
        // ── 技术指标周期 ──
        Variable {
            name: "macd_fast".into(),
            var_type: "number".into(),
            value: serde_json::json!(12),
            description: Some("MACD 快线周期".into()),
            is_secret: false,
        },
        Variable {
            name: "macd_slow".into(),
            var_type: "number".into(),
            value: serde_json::json!(26),
            description: Some("MACD 慢线周期".into()),
            is_secret: false,
        },
        Variable {
            name: "macd_signal".into(),
            var_type: "number".into(),
            value: serde_json::json!(9),
            description: Some("MACD 信号线周期".into()),
            is_secret: false,
        },
        Variable {
            name: "boll_period".into(),
            var_type: "number".into(),
            value: serde_json::json!(20),
            description: Some("布林带周期".into()),
            is_secret: false,
        },
        Variable {
            name: "boll_stddev".into(),
            var_type: "number".into(),
            value: serde_json::json!(2.0),
            description: Some("布林带标准差倍数".into()),
            is_secret: false,
        },
        Variable {
            name: "volume_lookback".into(),
            var_type: "number".into(),
            value: serde_json::json!(5),
            description: Some("均量计算回看周期 (交易日)".into()),
            is_secret: false,
        },
        Variable {
            name: "volume_surge_ratio".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.5),
            description: Some("放量阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "volume_shrink_ratio".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.7),
            description: Some("缩量阈值".into()),
            is_secret: false,
        },
        // ── 技术指标补充参数 ──
        Variable {
            name: "atr_period".into(),
            var_type: "number".into(),
            value: serde_json::json!(14),
            description: Some("ATR 计算周期（默认 14）".into()),
            is_secret: false,
        },
        Variable {
            name: "kdj_n".into(),
            var_type: "number".into(),
            value: serde_json::json!(9),
            description: Some("KDJ 计算周期 N".into()),
            is_secret: false,
        },
        Variable {
            name: "fill_missing_method".into(),
            var_type: "enum".into(),
            value: serde_json::json!("forward"),
            description: Some("数据清洗缺失值填充: forward / zero / drop".into()),
            is_secret: false,
        },
        Variable {
            name: "breakout_volume_threshold".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.5),
            description: Some("突破放量倍数阈值".into()),
            is_secret: false,
        },
        // ── 推荐器策略参数 ──
        Variable {
            name: "trend_kline_limit".into(),
            var_type: "number".into(),
            value: serde_json::json!(250),
            description: Some("趋势策略读取 K 线上限".into()),
            is_secret: false,
        },
        Variable {
            name: "trend_amount_ratio_min".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.8),
            description: Some("趋势策略最低量比".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_rsi_short_max".into(),
            var_type: "number".into(),
            value: serde_json::json!(35.0),
            description: Some("超跌反弹 RSI 判断上限".into()),
            is_secret: false,
        },
        // ── 趋势策略入池门槛与共享乘数（2026-09-12 补齐） ──
        // 这批 key 一直在 trend.rs 里以 `read_f64(vars, key, DEFAULT_*)` 读取，但 seed
        // 从未定义它们 → DB 变量表里不存在 → 前端 resolve 分组也无从列出 →
        // 用户只能改代码常量。其中 trend_high_20_threshold 与 trend_amount_ratio_min
        // 直接构成**超短线的入池条件**（现价 ≥ 5 日高 × high_20_threshold，量比 ≥ amount_ratio_min），
        // 门槛过严时「拿不到候选」在面板上完全无法自查与调整。
        // 默认值与 trend.rs 顶部 DEFAULT_* 常量逐一对齐，缺失时行为与修复前完全一致。
        Variable {
            name: "trend_high_20_threshold".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.97),
            description: Some(
                "趋势策略：现价需 ≥ 前高 × 该比例（超短线取 5 日高，短线取 20 日高）。\
                 调低=放宽入池（更多候选），调高=更严格"
                    .into(),
            ),
            is_secret: false,
        },
        Variable {
            name: "trend_short_ma20_tolerance".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.985),
            description: Some("趋势策略(短线)：现价需 ≥ MA20 × 该比例".into()),
            is_secret: false,
        },
        Variable {
            name: "trend_ma60_threshold".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.985),
            description: Some("趋势策略(中线)：现价需 ≥ MA60 × 该比例".into()),
            is_secret: false,
        },
        Variable {
            name: "trend_high_60_threshold".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.94),
            description: Some("趋势策略(中线)：现价需 ≥ 60 日高 × 该比例".into()),
            is_secret: false,
        },
        Variable {
            name: "trend_entry_tightness".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.0),
            description: Some("趋势策略入场区间乘数（1.0=标准，>1 更宽松，<1 更紧）".into()),
            is_secret: false,
        },
        Variable {
            name: "trend_stop_mult".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.0),
            description: Some("趋势策略止损距离乘数（1.0=标准，<1 止损更紧）".into()),
            is_secret: false,
        },
        Variable {
            name: "trend_target_mult".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.0),
            description: Some("趋势策略目标涨幅乘数（1.0=标准，>1 更激进）".into()),
            is_secret: false,
        },
        Variable {
            name: "trend_position_adj".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.0),
            description: Some("趋势策略基础仓位乘数（1.0=标准，0.5=半仓）".into()),
            is_secret: false,
        },
        Variable {
            name: "trend_conf_consistency".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.85),
            description: Some("趋势策略置信度-一致性权重（越大越依赖多信号一致）".into()),
            is_secret: false,
        },
        Variable {
            name: "trend_conf_signal".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.7),
            description: Some("趋势策略置信度-信号权重".into()),
            is_secret: false,
        },
        Variable {
            name: "trend_conf_market".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.0),
            description: Some("趋势策略置信度-市况权重".into()),
            is_secret: false,
        },
        // ── 推荐器参数 · 门槛与过滤条件（决定「是否入池」——超短线拿不到候选时优先查这里）（2026-09-12 批补） ──
        Variable {
            name: "cap_kline_mom_5_max".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.1),
            description: Some("资金流策略：K 线 5 日动量上限（超过视为短期过热）".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_kline_mom_5_min".into(),
            var_type: "number".into(),
            value: serde_json::json!(-0.02),
            description: Some("资金流策略：K 线 5 日动量下限（低于视为动能不足）".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_kline_vol_ratio_min".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.5),
            description: Some("资金流策略：K 线量比下限".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_long_main_inflow_min".into(),
            var_type: "number".into(),
            value: serde_json::json!(100),
            description: Some("资金流策略：主力净流入下限（万元）｜长线".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_long_nb_ratio_min".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.1),
            description: Some("资金流策略：北向持股占比下限（%）｜长线".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_mid_main_inflow_min".into(),
            var_type: "number".into(),
            value: serde_json::json!(500),
            description: Some("资金流策略：主力净流入下限（万元）｜中线".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_mid_nb_ratio_min".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.3),
            description: Some("资金流策略：北向持股占比下限（%）｜中线".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_short_main_inflow_min".into(),
            var_type: "number".into(),
            value: serde_json::json!(200),
            description: Some("资金流策略：主力净流入下限（万元）｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_short_turnover_min".into(),
            var_type: "number".into(),
            value: serde_json::json!(2),
            description: Some("资金流策略：换手率下限（%）｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_ultra_short_dt_net_min".into(),
            var_type: "number".into(),
            value: serde_json::json!(100),
            description: Some("资金流策略：龙虎榜净买额下限（万元）｜超短线".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_ultra_short_turnover_min".into(),
            var_type: "number".into(),
            value: serde_json::json!(5),
            description: Some("资金流策略：换手率下限（%）｜超短线".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_dd_min".into(),
            var_type: "number".into(),
            value: serde_json::json!(20),
            description: Some("超跌反弹策略：最低回撤幅度（%）".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_min_divergence_strength".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.3),
            description: Some("超跌反弹策略：最小背离强度".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_rsi_mid_max".into(),
            var_type: "number".into(),
            value: serde_json::json!(50),
            description: Some("超跌反弹策略：RSI 上限（中线，低于视为超跌）".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_rsi_mid_max_divergence".into(),
            var_type: "number".into(),
            value: serde_json::json!(55),
            description: Some("超跌反弹策略：RSI 背离判定上限（中线）".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_rsi_mid_period".into(),
            var_type: "number".into(),
            value: serde_json::json!(30),
            description: Some("超跌反弹策略：RSI 计算周期（中线）".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_rsi_period".into(),
            var_type: "number".into(),
            value: serde_json::json!(6),
            description: Some("超跌反弹策略：RSI 计算周期".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_rsi_short_max_divergence".into(),
            var_type: "number".into(),
            value: serde_json::json!(40),
            description: Some("超跌反弹策略：RSI 背离判定上限（短线）".into()),
            is_secret: false,
        },
        Variable {
            name: "serenity_growth_exempt_pct".into(),
            var_type: "number".into(),
            value: serde_json::json!(50),
            description: Some("Serenity 严选策略：高成长豁免阈值（%）".into()),
            is_secret: false,
        },
        Variable {
            name: "serenity_max_12m_gain_pct".into(),
            var_type: "number".into(),
            value: serde_json::json!(300),
            description: Some("Serenity 严选策略：近 12 个月最大涨幅上限（%）".into()),
            is_secret: false,
        },
        Variable {
            name: "serenity_max_3m_gain_pct".into(),
            var_type: "number".into(),
            value: serde_json::json!(80),
            description: Some("Serenity 严选策略：近 3 个月最大涨幅上限（%）".into()),
            is_secret: false,
        },
        Variable {
            name: "serenity_max_debt_ratio".into(),
            var_type: "number".into(),
            value: serde_json::json!(70),
            description: Some("Serenity 严选策略：资产负债率上限（%）".into()),
            is_secret: false,
        },
        Variable {
            name: "serenity_max_pb".into(),
            var_type: "number".into(),
            value: serde_json::json!(10),
            description: Some("Serenity 严选策略：市净率上限".into()),
            is_secret: false,
        },
        Variable {
            name: "serenity_max_pe".into(),
            var_type: "number".into(),
            value: serde_json::json!(100),
            description: Some("Serenity 严选策略：市盈率上限".into()),
            is_secret: false,
        },
        Variable {
            name: "serenity_min_gross_margin".into(),
            var_type: "number".into(),
            value: serde_json::json!(30),
            description: Some("Serenity 严选策略：最低毛利率（%）".into()),
            is_secret: false,
        },
        Variable {
            name: "serenity_min_revenue_growth".into(),
            var_type: "number".into(),
            value: serde_json::json!(5),
            description: Some("Serenity 严选策略：最低营收增速（%）".into()),
            is_secret: false,
        },
        Variable {
            name: "val_long_pb_max".into(),
            var_type: "number".into(),
            value: serde_json::json!(6),
            description: Some("价值策略：市净率上限｜长线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_long_pe_max".into(),
            var_type: "number".into(),
            value: serde_json::json!(35),
            description: Some("价值策略：市盈率上限｜长线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_mid_pb_max".into(),
            var_type: "number".into(),
            value: serde_json::json!(8),
            description: Some("价值策略：市净率上限｜中线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_mid_pe_max".into(),
            var_type: "number".into(),
            value: serde_json::json!(40),
            description: Some("价值策略：市盈率上限｜中线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_short_pe_max".into(),
            var_type: "number".into(),
            value: serde_json::json!(50),
            description: Some("价值策略：市盈率上限｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_ultra_short_pe_max".into(),
            var_type: "number".into(),
            value: serde_json::json!(60),
            description: Some("价值策略：市盈率上限｜超短线".into()),
            is_secret: false,
        },
        // ── 推荐器参数 · 置信度分量与权重（决定打分尺度）（2026-09-12 批补） ──
        Variable {
            name: "cap_conf_consistency".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.8),
            description: Some("资金流策略：置信度·一致性分量权重".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_conf_direction".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.7),
            description: Some("资金流策略：置信度·方向分量权重".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_conf_market".into(),
            var_type: "number".into(),
            value: serde_json::json!(0),
            description: Some("资金流策略：置信度·市场分量权重（0=不参与打分）".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_conf_signal".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.7),
            description: Some("资金流策略：置信度·信号分量权重".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_conf_base".into(),
            var_type: "number".into(),
            value: serde_json::json!(1),
            description: Some("超跌反弹策略：置信度基准系数".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_conf_consistency".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.7),
            description: Some("超跌反弹策略：置信度·一致性分量权重".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_conf_direction".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.6),
            description: Some("超跌反弹策略：置信度·方向分量权重".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_conf_market".into(),
            var_type: "number".into(),
            value: serde_json::json!(0),
            description: Some("超跌反弹策略：置信度·市场分量权重（0=不参与打分）".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_conf_signal".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.8),
            description: Some("超跌反弹策略：置信度·信号分量权重".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_divergence_bonus".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.1),
            description: Some("超跌反弹策略：背离形态的置信度加成".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_pattern_bonus".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.05),
            description: Some("超跌反弹策略：K 线形态的置信度加成".into()),
            is_secret: false,
        },
        Variable {
            name: "syn_conf_base".into(),
            var_type: "number".into(),
            value: serde_json::json!(1),
            description: Some("合成兜底策略：置信度基准系数".into()),
            is_secret: false,
        },
        Variable {
            name: "syn_conf_consistency".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.45),
            description: Some("合成兜底策略：置信度·一致性分量权重".into()),
            is_secret: false,
        },
        Variable {
            name: "syn_conf_direction".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.4),
            description: Some("合成兜底策略：置信度·方向分量权重".into()),
            is_secret: false,
        },
        Variable {
            name: "syn_conf_market".into(),
            var_type: "number".into(),
            value: serde_json::json!(0),
            description: Some("合成兜底策略：置信度·市场分量权重（0=不参与打分）".into()),
            is_secret: false,
        },
        Variable {
            name: "syn_conf_signal".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.4),
            description: Some("合成兜底策略：置信度·信号分量权重".into()),
            is_secret: false,
        },
        Variable {
            name: "syn_min_confidence".into(),
            var_type: "number".into(),
            value: serde_json::json!(25),
            description: Some("合成兜底策略：最低置信度门槛".into()),
            is_secret: false,
        },
        Variable {
            name: "val_conf_consistency".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.75),
            description: Some("价值策略：置信度·一致性分量权重".into()),
            is_secret: false,
        },
        Variable {
            name: "val_conf_direction".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.6),
            description: Some("价值策略：置信度·方向分量权重".into()),
            is_secret: false,
        },
        Variable {
            name: "val_conf_market".into(),
            var_type: "number".into(),
            value: serde_json::json!(0),
            description: Some("价值策略：置信度·市场分量权重（0=不参与打分）".into()),
            is_secret: false,
        },
        Variable {
            name: "val_conf_signal".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.7),
            description: Some("价值策略：置信度·信号分量权重".into()),
            is_secret: false,
        },
        Variable {
            name: "wl_conf_base".into(),
            var_type: "number".into(),
            value: serde_json::json!(1),
            description: Some("自选股策略：置信度基准系数".into()),
            is_secret: false,
        },
        Variable {
            name: "wl_conf_consistency".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.55),
            description: Some("自选股策略：置信度·一致性分量权重".into()),
            is_secret: false,
        },
        Variable {
            name: "wl_conf_direction".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.5),
            description: Some("自选股策略：置信度·方向分量权重".into()),
            is_secret: false,
        },
        Variable {
            name: "wl_conf_market".into(),
            var_type: "number".into(),
            value: serde_json::json!(0),
            description: Some("自选股策略：置信度·市场分量权重（0=不参与打分）".into()),
            is_secret: false,
        },
        Variable {
            name: "wl_conf_signal".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.5),
            description: Some("自选股策略：置信度·信号分量权重".into()),
            is_secret: false,
        },
        Variable {
            name: "wl_min_confidence".into(),
            var_type: "number".into(),
            value: serde_json::json!(30),
            description: Some("自选股策略：最低置信度门槛".into()),
            is_secret: false,
        },
        // ── 推荐器参数 · 仓位与价格区间（2026-09-12 批补） ──
        Variable {
            name: "cap_long_base_pos".into(),
            var_type: "number".into(),
            value: serde_json::json!(10),
            description: Some("资金流策略：基准仓位（%）｜长线".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_long_entry_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.05),
            description: Some("资金流策略：入场区间上沿（现价 × 该比例）｜长线".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_long_entry_low".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.95),
            description: Some("资金流策略：入场区间下沿（现价 × 该比例）｜长线".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_long_stop".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.88),
            description: Some("资金流策略：止损位（现价 × 该比例）｜长线".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_long_target".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.3),
            description: Some("资金流策略：目标位（现价 × 该比例）｜长线".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_mid_base_pos".into(),
            var_type: "number".into(),
            value: serde_json::json!(8),
            description: Some("资金流策略：基准仓位（%）｜中线".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_mid_entry_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.05),
            description: Some("资金流策略：入场区间上沿（现价 × 该比例）｜中线".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_mid_entry_low".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.95),
            description: Some("资金流策略：入场区间下沿（现价 × 该比例）｜中线".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_mid_stop".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.9),
            description: Some("资金流策略：止损位（现价 × 该比例）｜中线".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_mid_target".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.2),
            description: Some("资金流策略：目标位（现价 × 该比例）｜中线".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_short_base_pos".into(),
            var_type: "number".into(),
            value: serde_json::json!(5),
            description: Some("资金流策略：基准仓位（%）｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_short_entry_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.03),
            description: Some("资金流策略：入场区间上沿（现价 × 该比例）｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_short_entry_low".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.97),
            description: Some("资金流策略：入场区间下沿（现价 × 该比例）｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_short_stop".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.93),
            description: Some("资金流策略：止损位（现价 × 该比例）｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_short_target".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.1),
            description: Some("资金流策略：目标位（现价 × 该比例）｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_ultra_short_base_pos".into(),
            var_type: "number".into(),
            value: serde_json::json!(3),
            description: Some("资金流策略：基准仓位（%）｜超短线".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_ultra_short_entry_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.005),
            description: Some("资金流策略：入场区间上沿（现价 × 该比例）｜超短线".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_ultra_short_entry_low".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.998),
            description: Some("资金流策略：入场区间下沿（现价 × 该比例）｜超短线".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_ultra_short_stop".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.97),
            description: Some("资金流策略：止损位（现价 × 该比例）｜超短线".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_ultra_short_target".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.05),
            description: Some("资金流策略：目标位（现价 × 该比例）｜超短线".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_avg_amount_mult".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.2),
            description: Some("超跌反弹策略：成交额 / 均量 的最低倍数".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_mid_base_pos".into(),
            var_type: "number".into(),
            value: serde_json::json!(5),
            description: Some("超跌反弹策略：基准仓位（%）｜中线".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_mid_entry_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.05),
            description: Some("超跌反弹策略：入场区间上沿（现价 × 该比例）｜中线".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_mid_entry_low".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.95),
            description: Some("超跌反弹策略：入场区间下沿（现价 × 该比例）｜中线".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_mid_stop".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.88),
            description: Some("超跌反弹策略：止损位（现价 × 该比例）｜中线".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_mid_target".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.2),
            description: Some("超跌反弹策略：目标位（现价 × 该比例）｜中线".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_short_base_pos".into(),
            var_type: "number".into(),
            value: serde_json::json!(3),
            description: Some("超跌反弹策略：基准仓位（%）｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_short_entry_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.03),
            description: Some("超跌反弹策略：入场区间上沿（现价 × 该比例）｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_short_entry_low".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.97),
            description: Some("超跌反弹策略：入场区间下沿（现价 × 该比例）｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_short_stop".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.93),
            description: Some("超跌反弹策略：止损位（现价 × 该比例）｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_short_target".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.08),
            description: Some("超跌反弹策略：目标位（现价 × 该比例）｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "serenity_base_position".into(),
            var_type: "number".into(),
            value: serde_json::json!(10),
            description: Some("Serenity 严选策略：基准仓位（%）".into()),
            is_secret: false,
        },
        Variable {
            name: "serenity_entry_range".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.05),
            description: Some("Serenity 严选策略：入场区间半宽（现价 ± 该比例）".into()),
            is_secret: false,
        },
        Variable {
            name: "serenity_stop_mult".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.8),
            description: Some("Serenity 严选策略：止损距离乘数（1.0=标准，<1 更紧）".into()),
            is_secret: false,
        },
        Variable {
            name: "serenity_target_mult".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.3),
            description: Some("Serenity 严选策略：目标涨幅乘数（1.0=标准，>1 更激进）".into()),
            is_secret: false,
        },
        Variable {
            name: "serenity_target_pe".into(),
            var_type: "number".into(),
            value: serde_json::json!(25),
            description: Some("Serenity 严选策略：目标市盈率".into()),
            is_secret: false,
        },
        Variable {
            name: "syn_long_base_pos".into(),
            var_type: "number".into(),
            value: serde_json::json!(8),
            description: Some("合成兜底策略：基准仓位（%）｜长线".into()),
            is_secret: false,
        },
        Variable {
            name: "syn_long_entry_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.05),
            description: Some("合成兜底策略：入场区间上沿（现价 × 该比例）｜长线".into()),
            is_secret: false,
        },
        Variable {
            name: "syn_long_entry_low".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.95),
            description: Some("合成兜底策略：入场区间下沿（现价 × 该比例）｜长线".into()),
            is_secret: false,
        },
        Variable {
            name: "syn_long_stop".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.88),
            description: Some("合成兜底策略：止损位（现价 × 该比例）｜长线".into()),
            is_secret: false,
        },
        Variable {
            name: "syn_long_target".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.25),
            description: Some("合成兜底策略：目标位（现价 × 该比例）｜长线".into()),
            is_secret: false,
        },
        Variable {
            name: "syn_mid_base_pos".into(),
            var_type: "number".into(),
            value: serde_json::json!(6),
            description: Some("合成兜底策略：基准仓位（%）｜中线".into()),
            is_secret: false,
        },
        Variable {
            name: "syn_mid_entry_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.03),
            description: Some("合成兜底策略：入场区间上沿（现价 × 该比例）｜中线".into()),
            is_secret: false,
        },
        Variable {
            name: "syn_mid_entry_low".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.97),
            description: Some("合成兜底策略：入场区间下沿（现价 × 该比例）｜中线".into()),
            is_secret: false,
        },
        Variable {
            name: "syn_mid_stop".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.92),
            description: Some("合成兜底策略：止损位（现价 × 该比例）｜中线".into()),
            is_secret: false,
        },
        Variable {
            name: "syn_mid_target".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.15),
            description: Some("合成兜底策略：目标位（现价 × 该比例）｜中线".into()),
            is_secret: false,
        },
        Variable {
            name: "syn_short_base_pos".into(),
            var_type: "number".into(),
            value: serde_json::json!(4),
            description: Some("合成兜底策略：基准仓位（%）｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "syn_short_entry_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.01),
            description: Some("合成兜底策略：入场区间上沿（现价 × 该比例）｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "syn_short_entry_low".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.99),
            description: Some("合成兜底策略：入场区间下沿（现价 × 该比例）｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "syn_short_stop".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.96),
            description: Some("合成兜底策略：止损位（现价 × 该比例）｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "syn_short_target".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.06),
            description: Some("合成兜底策略：目标位（现价 × 该比例）｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "syn_ultra_short_base_pos".into(),
            var_type: "number".into(),
            value: serde_json::json!(2),
            description: Some("合成兜底策略：基准仓位（%）｜超短线".into()),
            is_secret: false,
        },
        Variable {
            name: "syn_ultra_short_entry_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.005),
            description: Some("合成兜底策略：入场区间上沿（现价 × 该比例）｜超短线".into()),
            is_secret: false,
        },
        Variable {
            name: "syn_ultra_short_entry_low".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.998),
            description: Some("合成兜底策略：入场区间下沿（现价 × 该比例）｜超短线".into()),
            is_secret: false,
        },
        Variable {
            name: "syn_ultra_short_stop".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.98),
            description: Some("合成兜底策略：止损位（现价 × 该比例）｜超短线".into()),
            is_secret: false,
        },
        Variable {
            name: "syn_ultra_short_target".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.03),
            description: Some("合成兜底策略：目标位（现价 × 该比例）｜超短线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_long_base_pos".into(),
            var_type: "number".into(),
            value: serde_json::json!(10),
            description: Some("价值策略：基准仓位（%）｜长线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_long_entry_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.05),
            description: Some("价值策略：入场区间上沿（现价 × 该比例）｜长线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_long_entry_low".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.93),
            description: Some("价值策略：入场区间下沿（现价 × 该比例）｜长线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_long_ma60_mult".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.9),
            description: Some("价值策略：现价 / MA60 的最低比值｜长线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_long_stop".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.85),
            description: Some("价值策略：止损位（现价 × 该比例）｜长线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_long_target".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.3),
            description: Some("价值策略：目标位（现价 × 该比例）｜长线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_mid_base_pos".into(),
            var_type: "number".into(),
            value: serde_json::json!(8),
            description: Some("价值策略：基准仓位（%）｜中线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_mid_entry_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.05),
            description: Some("价值策略：入场区间上沿（现价 × 该比例）｜中线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_mid_entry_low".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.95),
            description: Some("价值策略：入场区间下沿（现价 × 该比例）｜中线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_mid_stop".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.88),
            description: Some("价值策略：止损位（现价 × 该比例）｜中线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_mid_target".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.2),
            description: Some("价值策略：目标位（现价 × 该比例）｜中线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_short_base_pos".into(),
            var_type: "number".into(),
            value: serde_json::json!(5),
            description: Some("价值策略：基准仓位（%）｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_short_entry_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.02),
            description: Some("价值策略：入场区间上沿（现价 × 该比例）｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_short_entry_low".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.98),
            description: Some("价值策略：入场区间下沿（现价 × 该比例）｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_short_ma_mult".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.005),
            description: Some("价值策略：现价 / 均线 的最低比值｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_short_stop".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.93),
            description: Some("价值策略：止损位（现价 × 该比例）｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_short_target".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.1),
            description: Some("价值策略：目标位（现价 × 该比例）｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_ultra_short_base_pos".into(),
            var_type: "number".into(),
            value: serde_json::json!(3),
            description: Some("价值策略：基准仓位（%）｜超短线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_ultra_short_entry_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.005),
            description: Some("价值策略：入场区间上沿（现价 × 该比例）｜超短线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_ultra_short_entry_low".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.998),
            description: Some("价值策略：入场区间下沿（现价 × 该比例）｜超短线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_ultra_short_ma_mult".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.005),
            description: Some("价值策略：现价 / 均线 的最低比值｜超短线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_ultra_short_stop".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.97),
            description: Some("价值策略：止损位（现价 × 该比例）｜超短线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_ultra_short_target".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.03),
            description: Some("价值策略：目标位（现价 × 该比例）｜超短线".into()),
            is_secret: false,
        },
        Variable {
            name: "wl_long_base_pos".into(),
            var_type: "number".into(),
            value: serde_json::json!(8),
            description: Some("自选股策略：基准仓位（%）｜长线".into()),
            is_secret: false,
        },
        Variable {
            name: "wl_long_entry_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.05),
            description: Some("自选股策略：入场区间上沿（现价 × 该比例）｜长线".into()),
            is_secret: false,
        },
        Variable {
            name: "wl_long_entry_low".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.95),
            description: Some("自选股策略：入场区间下沿（现价 × 该比例）｜长线".into()),
            is_secret: false,
        },
        Variable {
            name: "wl_long_stop".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.88),
            description: Some("自选股策略：止损位（现价 × 该比例）｜长线".into()),
            is_secret: false,
        },
        Variable {
            name: "wl_long_target".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.25),
            description: Some("自选股策略：目标位（现价 × 该比例）｜长线".into()),
            is_secret: false,
        },
        Variable {
            name: "wl_mid_base_pos".into(),
            var_type: "number".into(),
            value: serde_json::json!(6),
            description: Some("自选股策略：基准仓位（%）｜中线".into()),
            is_secret: false,
        },
        Variable {
            name: "wl_mid_entry_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.03),
            description: Some("自选股策略：入场区间上沿（现价 × 该比例）｜中线".into()),
            is_secret: false,
        },
        Variable {
            name: "wl_mid_entry_low".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.97),
            description: Some("自选股策略：入场区间下沿（现价 × 该比例）｜中线".into()),
            is_secret: false,
        },
        Variable {
            name: "wl_mid_stop".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.92),
            description: Some("自选股策略：止损位（现价 × 该比例）｜中线".into()),
            is_secret: false,
        },
        Variable {
            name: "wl_mid_target".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.15),
            description: Some("自选股策略：目标位（现价 × 该比例）｜中线".into()),
            is_secret: false,
        },
        Variable {
            name: "wl_short_base_pos".into(),
            var_type: "number".into(),
            value: serde_json::json!(4),
            description: Some("自选股策略：基准仓位（%）｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "wl_short_entry_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.01),
            description: Some("自选股策略：入场区间上沿（现价 × 该比例）｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "wl_short_entry_low".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.99),
            description: Some("自选股策略：入场区间下沿（现价 × 该比例）｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "wl_short_stop".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.96),
            description: Some("自选股策略：止损位（现价 × 该比例）｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "wl_short_target".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.06),
            description: Some("自选股策略：目标位（现价 × 该比例）｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "wl_ultra_short_base_pos".into(),
            var_type: "number".into(),
            value: serde_json::json!(2),
            description: Some("自选股策略：基准仓位（%）｜超短线".into()),
            is_secret: false,
        },
        Variable {
            name: "wl_ultra_short_entry_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.005),
            description: Some("自选股策略：入场区间上沿（现价 × 该比例）｜超短线".into()),
            is_secret: false,
        },
        Variable {
            name: "wl_ultra_short_entry_low".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.998),
            description: Some("自选股策略：入场区间下沿（现价 × 该比例）｜超短线".into()),
            is_secret: false,
        },
        Variable {
            name: "wl_ultra_short_stop".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.98),
            description: Some("自选股策略：止损位（现价 × 该比例）｜超短线".into()),
            is_secret: false,
        },
        Variable {
            name: "wl_ultra_short_target".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.03),
            description: Some("自选股策略：目标位（现价 × 该比例）｜超短线".into()),
            is_secret: false,
        },
        // ── 推荐器参数 · 数据窗口与限流（2026-09-12 批补） ──
        Variable {
            name: "rev_avg_amount_period".into(),
            var_type: "number".into(),
            value: serde_json::json!(5),
            description: Some("超跌反弹策略：均量计算天数".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_dd_period".into(),
            var_type: "number".into(),
            value: serde_json::json!(250),
            description: Some("超跌反弹策略：回撤回看天数".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_divergence_lookback".into(),
            var_type: "number".into(),
            value: serde_json::json!(14),
            description: Some("超跌反弹策略：背离回看天数".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_kline_limit".into(),
            var_type: "number".into(),
            value: serde_json::json!(250),
            description: Some("超跌反弹策略：K 线读取上限（条）".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_min_kline_len".into(),
            var_type: "number".into(),
            value: serde_json::json!(30),
            description: Some("超跌反弹策略：K 线最少条数（不足则跳过该股）".into()),
            is_secret: false,
        },
        Variable {
            name: "val_long_kline_limit".into(),
            var_type: "number".into(),
            value: serde_json::json!(70),
            description: Some("价值策略：K 线读取上限（条）｜长线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_long_ma_period".into(),
            var_type: "number".into(),
            value: serde_json::json!(60),
            description: Some("价值策略：均线计算周期｜长线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_short_kline_limit".into(),
            var_type: "number".into(),
            value: serde_json::json!(30),
            description: Some("价值策略：K 线读取上限（条）｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_short_ma_period".into(),
            var_type: "number".into(),
            value: serde_json::json!(20),
            description: Some("价值策略：均线计算周期｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_short_min_kline_len".into(),
            var_type: "number".into(),
            value: serde_json::json!(20),
            description: Some("价值策略：K 线最少条数（不足则跳过该股）｜短线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_ultra_short_kline_limit".into(),
            var_type: "number".into(),
            value: serde_json::json!(10),
            description: Some("价值策略：K 线读取上限（条）｜超短线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_ultra_short_ma_period".into(),
            var_type: "number".into(),
            value: serde_json::json!(10),
            description: Some("价值策略：均线计算周期｜超短线".into()),
            is_secret: false,
        },
        Variable {
            name: "val_ultra_short_min_kline_len".into(),
            var_type: "number".into(),
            value: serde_json::json!(5),
            description: Some("价值策略：K 线最少条数（不足则跳过该股）｜超短线".into()),
            is_secret: false,
        },
        // ── 基本面修正阈值 ──
        Variable {
            name: "val_pe_low".into(),
            var_type: "number".into(),
            value: serde_json::json!(15.0),
            description: Some("基本面修正 PE 低估阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "val_pe_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(50.0),
            description: Some("基本面修正 PE 高估阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "val_pb_low".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.0),
            description: Some("基本面修正 PB 低估阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "val_pb_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(6.0),
            description: Some("基本面修正 PB 高估阈值".into()),
            is_secret: false,
        },
        // ── 组合风控 HHI ──
        Variable {
            name: "risk_hhi_concentrated".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.25),
            description: Some("组合 HHI 高度集中阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "risk_hhi_medium".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.15),
            description: Some("组合 HHI 中度集中阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "risk_divers_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(8.0),
            description: Some("组合有效股票数充分分散阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "risk_divers_medium".into(),
            var_type: "number".into(),
            value: serde_json::json!(4.0),
            description: Some("组合有效股票数适度分散阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "analysis_dry_run".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(false),
            description: Some("干跑模式：不调用 LLM，用 mock 输出验证流程".into()),
            is_secret: false,
        },
        // ── 业绩超预期分级阈值 ──
        Variable {
            name: "earnings_th_huge_pos".into(),
            var_type: "number".into(),
            value: serde_json::json!(50.0),
            description: Some("大幅超预期下界 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "earnings_th_strong_pos".into(),
            var_type: "number".into(),
            value: serde_json::json!(20.0),
            description: Some("强超预期下界 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "earnings_th_mild_pos".into(),
            var_type: "number".into(),
            value: serde_json::json!(5.0),
            description: Some("略超预期下界 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "earnings_th_mild_neg".into(),
            var_type: "number".into(),
            value: serde_json::json!(-5.0),
            description: Some("略低于预期下界 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "earnings_th_strong_neg".into(),
            var_type: "number".into(),
            value: serde_json::json!(-20.0),
            description: Some("强低于预期下界 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "earnings_th_huge_neg".into(),
            var_type: "number".into(),
            value: serde_json::json!(-50.0),
            description: Some("大幅低于预期下界 (%)".into()),
            is_secret: false,
        },
        // ── 质押风险分级阈值 ──
        Variable {
            name: "pledge_warning_line".into(),
            var_type: "number".into(),
            value: serde_json::json!(50.0),
            description: Some("大股东质押比例预警线 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "pledge_liquidation_line".into(),
            var_type: "number".into(),
            value: serde_json::json!(70.0),
            description: Some("大股东质押比例平仓线 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "pledge_medium_line".into(),
            var_type: "number".into(),
            value: serde_json::json!(30.0),
            description: Some("大股东质押中风险阈值 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "pledge_low_line".into(),
            var_type: "number".into(),
            value: serde_json::json!(10.0),
            description: Some("大股东质押低风险阈值 (%)".into()),
            is_secret: false,
        },
        // ── 蒙特卡洛模拟默认参数 ──
        Variable {
            name: "mc_default_price".into(),
            var_type: "number".into(),
            value: serde_json::json!(10.0),
            description: Some("蒙特卡洛模拟默认价格".into()),
            is_secret: false,
        },
        Variable {
            name: "mc_default_return".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.08),
            description: Some("蒙特卡洛模拟默认年化收益".into()),
            is_secret: false,
        },
        Variable {
            name: "mc_default_volatility".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.3),
            description: Some("蒙特卡洛模拟默认年化波动率".into()),
            is_secret: false,
        },
        Variable {
            name: "mc_default_days".into(),
            var_type: "number".into(),
            value: serde_json::json!(30),
            description: Some("蒙特卡洛模拟默认天数".into()),
            is_secret: false,
        },
        Variable {
            name: "mc_default_simulations".into(),
            var_type: "number".into(),
            value: serde_json::json!(1000),
            description: Some("蒙特卡洛模拟默认路径数".into()),
            is_secret: false,
        },
        // ── 行业内估值/增长对比阈值 ──
        Variable {
            name: "industry_pe_cheap".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.0),
            description: Some("行业内 PE 相对低估阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "industry_pe_expensive".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.5),
            description: Some("行业内 PE 相对高估阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "industry_growth_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.2),
            description: Some("行业内高增长阈值".into()),
            is_secret: false,
        },
        // ── 涨停潜力评分 ──
        Variable {
            name: "limit_pct_main".into(),
            var_type: "number".into(),
            value: serde_json::json!(10.0),
            description: Some("主板涨停幅度 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "limit_pct_star".into(),
            var_type: "number".into(),
            value: serde_json::json!(20.0),
            description: Some("创业板/科创板涨停幅度 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "limit_pct_bj".into(),
            var_type: "number".into(),
            value: serde_json::json!(30.0),
            description: Some("北交所涨停幅度 (%)".into()),
            is_secret: false,
        },
        // ── 反思复盘参数 ──
        Variable {
            name: "actual_outcome".into(),
            var_type: "string".into(),
            value: serde_json::json!(""),
            description: Some("实际走势结果，非空时切换反思模式".into()),
            is_secret: false,
        },
        Variable {
            name: "reflection_depth".into(),
            var_type: "string".into(),
            value: serde_json::json!("light"),
            description: Some("反思深度：light(简要) / deep(详细推理链)".into()),
            is_secret: false,
        },
        // ── 历史反思教训注入 ──
        Variable {
            name: "stock_lessons".into(),
            var_type: "string".into(),
            value: serde_json::json!("（暂无历史反思）"),
            description: Some("该股最近 90 天的反思教训,由 runtime 注入".into()),
            is_secret: false,
        },
        // ══════════════════════════════════════════════════════════════
        // ── 决策参数（portfolio-mgr）：可配置 + 可被反思/演进优化 ──
        // ══════════════════════════════════════════════════════════════
        // 修复背景（2026-09-11）：
        //   此前 seed_stock_analysis.rs 的 portfolio-mgr input_mapping 已经映射
        //   了 24 个决策参数（action_* / pos_* / risk_* / cost_pct）的「同名变量」，
        //   但变量表从未定义过它们，形成四处断链：
        //     ① context.variables 查不到 → rhai 的 present() 恒假
        //        → 全部静默走 rhai 内硬编码默认值，「可配置」形同虚设；
        //     ② 前端 StockAnalysisConfigPanel 的 portfolio_mgr_action /
        //        portfolio_mgr_risk 两个分组因变量不存在被 .filter(Boolean)
        //        整组过滤 → 界面永久空白；
        //     ③ 反思产出的参数建议（apply_param_suggestions）按 param 名查变量，
        //        找不到即 tracing::warn 静默丢弃 → 反思优化 100% 失效；
        //     ④ 反思侧 PortfolioMgrParamSet 用 buy_threshold/cap_high 短名，
        //        与本表 action_buy_threshold/pos_cap_high 全名不一致。
        //   本节补齐变量定义，并配合 stock_analysis.rs::PARAM_ALIASES 打通命名。
        //
        //   覆盖层级（优先级低 → 高）：
        //     rhai 内置默认值 < 本节变量值 < 反思建议应用（apply_param_suggestions）
        //   运行时实际生效值会写入决策输出的 effective_params 字段，供反思观测归因。
        // ── 市况先验（prior 由市况方向派生，不再是分类置信度）──
        Variable {
            name: "regime_prior_bull".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.55),
            description: Some("牛市先验概率：无个股证据时对上涨的基础判断 (0-1)".into()),
            is_secret: false,
        },
        Variable {
            name: "regime_prior_sideways".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.50),
            description: Some("震荡市先验概率：无个股证据时对上涨的基础判断 (0-1)".into()),
            is_secret: false,
        },
        Variable {
            name: "regime_prior_bear".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.45),
            description: Some("熊市先验概率：无个股证据时对上涨的基础判断 (0-1)".into()),
            is_secret: false,
        },
        // ── 因子融合门（作用于后验概率封顶，位于先验与 action 分档之间）──
        // v33(2026-09-11): portfolio-mgr.rhai 早就有 `if present(trader_cap_min_weight)`
        // 守卫（f7 权重低于该值时，交易员的看空信号不再封顶后验概率），但变量表从未
        // 定义该名、input_mapping 也无同名映射 → present() 恒假，永远走硬编码 0.08。
        // 属「配置项空接线」同型缺陷，本次补齐定义使其真正可配置、可被反思优化。
        Variable {
            name: "trader_cap_min_weight".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.08),
            description: Some(
                "交易员因子权重门 (0-1)：f7 权重低于该值时，其看空信号不再封顶后验概率".into(),
            ),
            is_secret: false,
        },
        // ── action 决策阈值（作用于 effective_posterior）──
        Variable {
            name: "action_buy_threshold".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.63),
            description: Some("买入动作阈值（后验概率下限，0-1）".into()),
            is_secret: false,
        },
        Variable {
            name: "action_increase_threshold".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.53),
            description: Some("增持动作阈值（后验概率下限，0-1）".into()),
            is_secret: false,
        },
        Variable {
            name: "action_hold_threshold".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.48),
            description: Some("持有动作阈值（后验概率下限，0-1）".into()),
            is_secret: false,
        },
        Variable {
            name: "action_watch_threshold".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.38),
            description: Some("观望动作阈值（后验概率下限，0-1）".into()),
            is_secret: false,
        },
        Variable {
            name: "action_reduce_threshold".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.30),
            description: Some("减持动作阈值（后验概率下限，0-1）".into()),
            is_secret: false,
        },
        // ── 仓位阈值 (%) ──
        Variable {
            name: "pos_buy_min".into(),
            var_type: "number".into(),
            value: serde_json::json!(15.0),
            description: Some(
                "买入所需的最小凯利意愿仓位 (%)，取未封顶的半凯利值，不受风险仓位上限影响".into(),
            ),
            is_secret: false,
        },
        Variable {
            name: "pos_increase_min".into(),
            var_type: "number".into(),
            value: serde_json::json!(10.0),
            description: Some(
                "增持所需的最小凯利意愿仓位 (%)，取未封顶的半凯利值，不受风险仓位上限影响".into(),
            ),
            is_secret: false,
        },
        // ── 风险等级仓位上限 (%) ──
        Variable {
            name: "pos_cap_extreme".into(),
            var_type: "number".into(),
            value: serde_json::json!(10.0),
            description: Some("极高风险仓位上限 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "pos_cap_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(35.0),
            description: Some("高风险仓位上限 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "pos_cap_mid".into(),
            var_type: "number".into(),
            value: serde_json::json!(50.0),
            description: Some("中风险仓位上限 (%)".into()),
            is_secret: false,
        },
        // ── 风险分类阈值：极高 ──
        Variable {
            name: "risk_debt_extreme".into(),
            var_type: "number".into(),
            value: serde_json::json!(85.0),
            description: Some("资产负债率极高风险阈值 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "risk_vol_extreme".into(),
            var_type: "number".into(),
            value: serde_json::json!(60.0),
            description: Some("年化波动率极高风险阈值 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "risk_sharpe_extreme".into(),
            var_type: "number".into(),
            value: serde_json::json!(-1.5),
            description: Some("夏普比率极高风险阈值".into()),
            is_secret: false,
        },
        // ── 风险分类阈值：高 ──
        Variable {
            name: "risk_vol_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(40.0),
            description: Some("年化波动率高风险阈值 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "risk_dd_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(45.0),
            description: Some("最大回撤高风险阈值 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "risk_roe_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(5.0),
            description: Some("ROE 高风险阈值 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "risk_debt_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(65.0),
            description: Some("资产负债率高风险阈值 (%)".into()),
            is_secret: false,
        },
        // ── 风险分类阈值：低 ──
        Variable {
            name: "risk_vol_low".into(),
            var_type: "number".into(),
            value: serde_json::json!(25.0),
            description: Some("年化波动率低风险阈值 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "risk_sharpe_low".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.5),
            description: Some("夏普比率低风险阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "risk_dd_low".into(),
            var_type: "number".into(),
            value: serde_json::json!(30.0),
            description: Some("最大回撤低风险阈值 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "risk_roe_low".into(),
            var_type: "number".into(),
            value: serde_json::json!(8.0),
            description: Some("ROE 低风险阈值 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "risk_debt_low".into(),
            var_type: "number".into(),
            value: serde_json::json!(55.0),
            description: Some("资产负债率低风险阈值 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "risk_growth_low".into(),
            var_type: "number".into(),
            value: serde_json::json!(3.0),
            description: Some("营收增速过低阈值 (%)".into()),
            is_secret: false,
        },
        // ── 〇-B v2 第 3 条：逐周期交易档位（可配置 + 可被反思优化）──
        // 此前这四对数字只硬编码在 `portfolio-mgr.rhai` 的 `sl_pct_for` / `tp_pct_for`，
        // 既不进设置面板、也不在反思的「可调参数清单」里 ⇒ 反思永远无法纠正档位错配。
        // 三处齐备（同 PORTFOLIO_MGR_TUNABLE_PARAMS 头部注释的规矩）：本表定义 + 脚本 present() 守卫
        // + 常量登记。⚠ **持有天数不在此列** —— 它是逐档成熟判定的基准，唯一权威源是
        //   `axagent_harness::holding_period::Period`，做成可调会一次改动就废掉全部按档统计。
        Variable {
            name: "sl_pct_ultra_short".into(),
            var_type: "number".into(),
            value: serde_json::json!(3.0),
            description: Some("止损档位（相对现价 %，超短线·缺省持有 2 交易日）".into()),
            is_secret: false,
        },
        Variable {
            name: "tp_pct_ultra_short".into(),
            var_type: "number".into(),
            value: serde_json::json!(5.0),
            description: Some("止盈档位（相对现价 %，超短线·缺省持有 2 交易日）".into()),
            is_secret: false,
        },
        Variable {
            name: "sl_pct_short".into(),
            var_type: "number".into(),
            value: serde_json::json!(5.0),
            description: Some("止损档位（相对现价 %，短线·缺省持有 5 交易日）".into()),
            is_secret: false,
        },
        Variable {
            name: "tp_pct_short".into(),
            var_type: "number".into(),
            value: serde_json::json!(10.0),
            description: Some("止盈档位（相对现价 %，短线·缺省持有 5 交易日）".into()),
            is_secret: false,
        },
        Variable {
            name: "sl_pct_mid".into(),
            var_type: "number".into(),
            value: serde_json::json!(8.0),
            description: Some("止损档位（相对现价 %，中线·缺省持有 28 交易日）".into()),
            is_secret: false,
        },
        Variable {
            name: "tp_pct_mid".into(),
            var_type: "number".into(),
            value: serde_json::json!(18.0),
            description: Some("止盈档位（相对现价 %，中线·缺省持有 28 交易日）".into()),
            is_secret: false,
        },
        Variable {
            name: "sl_pct_long".into(),
            var_type: "number".into(),
            value: serde_json::json!(12.0),
            description: Some("止损档位（相对现价 %，长线·缺省持有 90 交易日）".into()),
            is_secret: false,
        },
        Variable {
            name: "tp_pct_long".into(),
            var_type: "number".into(),
            value: serde_json::json!(30.0),
            description: Some("止盈档位（相对现价 %，长线·缺省持有 90 交易日）".into()),
            is_secret: false,
        },
        Variable {
            // Phase D-2：单标的风险预算 R（% of 组合）。仓位 = min(凯利%, 100×R/止损%)
            // ⇒ 止损被打掉时组合恰损失 R%。它替代「经验周期乘数」承担跨档差异。
            name: "risk_budget_pct".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.5),
            description: Some("单标的风险预算 R（% of 组合，止损被打掉时的组合损失上限）".into()),
            is_secret: false,
        },
        Variable {
            // 四周期科学化 Phase D：止损 = k1 × σ_daily × √持有天数。
            // 1.2 的含义是「止损放在该持有期典型位移的 1.2 倍处」，与标的波动、持有期自动同变；
            // 缺 σ 时脚本退回固定百分比档并在 stopSource 标 fallback_pct。
            name: "stop_vol_mult".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.2),
            description: Some("止损波动率乘数 k1（止损 = k1 × σ_daily × √持有天数）".into()),
            is_secret: false,
        },
        Variable {
            // 同上，止盈侧（默认 2.0 ⇒ 盈亏比 1:1.67，与旧固定档 5/3≈1.67 同量级，
            // 改动只把「不看波动的绝对数」换成「按波动的相对数」，不改变风险回报偏好）。
            name: "take_profit_vol_mult".into(),
            var_type: "number".into(),
            value: serde_json::json!(2.0),
            description: Some("止盈波动率乘数 k2（止盈 = k2 × σ_daily × √持有天数）".into()),
            is_secret: false,
        },
        Variable {
            // 四周期科学化 Phase C：逐档先验收缩强度 κ。
            // prior_h = (n_h·p_h + κ·p_pool)/(n_h+κ) —— κ→0 完全采信该档自身命中率，
            // κ→∞ 退回四档共用先验（即旧行为）。默认值与 `horizon_prior::DEFAULT_KAPPA` 同值。
            name: "horizon_prior_kappa".into(),
            var_type: "number".into(),
            value: serde_json::json!(axagent_analysis_engine::horizon_prior::DEFAULT_KAPPA),
            description: Some("逐档先验收缩强度 κ（越大越保守地退回全档合并基准）".into()),
            is_secret: false,
        },
        // 注：cost_pct（交易成本率）已在「金融模型补充参数」段定义，此处不重复。
    ]
}
