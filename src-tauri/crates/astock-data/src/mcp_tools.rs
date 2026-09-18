use serde_json::json;

// ── G14 DojoSDK 工具执行器（trait + 全局注册器） ─────────────────────────
//
// astock-data 是 implementor 层级，不能依赖 quant（consumer）/
// stock-analysis（implementor，但反向依赖会循环）/ tools（hybrid）。
// 因此 DojoSDK 工具的执行逻辑通过 trait 抽象，由 main crate 实现并注册。
//
// 调用顺序：
// 1. main crate 启动时调用 `register_dojo_sdk_executor(impl)` 注册实现
// 2. LLM 通过 MCP 协议调用 `dojo_*` / `sector_precomputed_*` 工具
// 3. `execute_mcp_tool` 命中 DojoSDK 工具时调用 `with_dojo_sdk_executor`
// 4. 实现内部路由到 quant / stock-analysis / tools 等具体 crate

/// DojoSDK 工具执行器 trait
///
/// 实现方需在 `execute` 中根据 `tool_name` 路由到具体的 SDK 功能。
/// 返回 JSON 字符串（与 `execute_mcp_tool` 一致）。
#[async_trait::async_trait]
pub trait DojoSdkExecutor: Send + Sync {
    async fn execute(
        &self,
        tool_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<String, String>;
}

/// 全局 DojoSdkExecutor 注册器（OnceLock 保证一次性注册）
static DOJO_SDK_EXECUTOR: std::sync::OnceLock<Box<dyn DojoSdkExecutor>> =
    std::sync::OnceLock::new();

/// 注册全局 DojoSdkExecutor（启动时调用一次）
///
/// 重复调用会被忽略（OnceLock 语义）。建议在 `init::services` 中注册。
pub fn register_dojo_sdk_executor(executor: Box<dyn DojoSdkExecutor>) {
    let _ = DOJO_SDK_EXECUTOR.set(executor);
}

/// 检查 DojoSdkExecutor 是否已注册
pub fn has_dojo_sdk_executor() -> bool {
    DOJO_SDK_EXECUTOR.get().is_some()
}

/// 判断工具名是否属于 DojoSDK 工具集
pub fn is_dojo_sdk_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "sector_precomputed_sector_alpha_factors_daily"
            | "dojo_run_quant_backtest"
            | "dojo_get_skill_content"
            | "dojo_list_skills"
            | "dojo_get_paper_portfolio"
            | "dojo_list_market_mainlines"
            | "dojo_create_plan"
            | "dojo_execute_plan"
            | "dojo_revise_plan"
    )
}

/// 委托 DojoSDK 工具到已注册的执行器
async fn with_dojo_sdk_executor(
    tool_name: &str,
    arguments: &serde_json::Value,
) -> Result<String, String> {
    match DOJO_SDK_EXECUTOR.get() {
        Some(executor) => executor.execute(tool_name, arguments).await,
        None => Err(format!(
            "DojoSDK 工具 '{tool_name}' 需要注册 DojoSdkExecutor 才能执行（启动时调用 register_dojo_sdk_executor）"
        )),
    }
}

pub fn stock_mcp_tools() -> Vec<serde_json::Value> {
    vec![
        json!({
            "name": "search_stock",
            "description": "搜索A股股票。keyword 必须是完整的中文名称（如'中国卫通'、'紫金矿业'）或 6 位数字代码（如'601698'），禁止传入拼音片段（如'zi'jin'、'zhongguo'）或中英混合片段。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "keyword": { "type": "string", "description": "完整中文名称或6位数字代码（如'中国卫通'、'601698'）。禁止拼音片段。" }
                },
                "required": ["keyword"]
            }
        }),
        json!({
            "name": "search_news",
            "description": "按关键词搜索财经新闻，用于验证催化剂/CapEx/行业趋势",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "keyword": { "type": "string", "description": "搜索关键词（如'英伟达 CapEx'、'HBM 产能扩张'）" },
                    "limit": { "type": "integer", "description": "返回条数（默认10）" }
                },
                "required": ["keyword"]
            }
        }),
        json!({
            "name": "get_stock_quote",
            "description": "获取A股实时行情（价格、涨跌幅、成交量等）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码，如600519" }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "get_stock_kline",
            "description": "获取A股历史K线数据（含日期、开高低收、成交量）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" },
                    "period": { "type": "string", "description": "周期：daily/weekly/monthly", "default": "daily" },
                    "limit": { "type": "integer", "description": "K线数量（1-500）", "default": 120 }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "get_stock_financials",
            "description": "获取A股财务报表（营收、净利润、EPS、ROE、毛利率等）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "get_fundamentals_report_markdown",
            "description": "获取基本面预聚合 Markdown 报告（健康度评分/估值带/安全边际/同比环比），供基本面分析师直接消费，避免重复计算基础比率",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "get_stock_news",
            "description": "获取A股相关新闻公告（含情绪评分）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" },
                    "limit": { "type": "integer", "description": "新闻数量", "default": 30 }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "get_stock_policy_news",
            "description": "获取政策相关新闻（基于股票所属行业做关键词搜索：政策/规划/通知/补贴）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" },
                    "limit": { "type": "integer", "description": "新闻数量", "default": 30 }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "get_stock_money_flow",
            "description": "获取A股资金流向（主力/超大单/大单/中单/小单净流入）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "get_social_sentiment",
            "description": "获取社交舆情数据（东方财富股吧帖子数/情感倾向/看多看空比例），用于情绪面分析师",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "get_stock_dragon_tiger",
            "description": "获取个股龙虎榜数据（营业部买卖、上榜原因）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "get_stock_margin_data",
            "description": "获取融资融券数据（融资买入额、余额、融券卖出量、余量）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "get_stock_sector_info",
            "description": "获取行业分类（申万一级/二级、概念板块标签）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "get_stock_north_bound",
            "description": "获取北向资金个股持仓（持股数量、占比）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "get_stock_lockup",
            "description": "获取限售解禁日程（解禁日期、股数、比例、股东名称）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "get_stock_lockup_bundle",
            "description": "获取解禁+大股东增减持+大宗交易聚合包（lockup-watcher 冷启动数据）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "get_stock_shareholder_trades",
            "description": "获取大股东增减持记录（变动类型、数量、均价、原因）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "get_stock_dividend_records",
            "description": "获取除权除息/分红送配记录",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "get_stock_research_reports",
            "description": "获取研报列表（机构、评级、目标价、EPS预测）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "get_stock_consensus_eps",
            "description": "获取机构一致预期EPS（一致预期EPS、目标价、评级）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "get_stock_concept_blocks",
            "description": "获取概念板块三维归属（行业/概念/地域）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "get_stock_announcements",
            "description": "获取巨潮全量公告（沪深北交所）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "get_stock_block_trades",
            "description": "获取大宗交易记录（交易日期、价格、数量、买方/卖方营业部）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "get_stock_institutional_visits",
            "description": "获取机构调研记录（调研日期、参与机构数、调研内容摘要）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "get_market_dragon_tiger",
            "description": "获取全市场龙虎榜（每日上榜股票+净买额排名）",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "get_hot_stocks",
            "description": "获取同花顺强势股（当日强势股+题材归因标签）",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "get_industry_ranking",
            "description": "获取行业横向排名（~90行业涨跌排名+领涨股）",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "get_cls_flash",
            "description": "获取财联社快讯（分钟级电报）",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "get_north_bound_flow",
            "description": "获取北向资金分钟级流向（沪深股通）",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "get_index_quotes",
            "description": "获取大盘指数行情（上证指数、深证成指、创业板指）",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "get_stock_peers",
            "description": "获取同行业可比公司估值（PE/PB/ROE/涨跌幅/市值）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "get_stock_option_pcr",
            "description": "获取期权PCR（看跌/看涨成交量和持仓量比率，市场情绪前瞻指标）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" }
                },
                "required": ["stock_code"]
            }
        }),
        // #4: 股权质押数据工具
        // 前置工具：LLM 在调用 detect_pledge_risk（tools/finance.rs）前应先调用本工具获取 pledge_pct。
        // 输出字段：pledge_ratio（大股东质押总比例%）、pledge_shares（质押股数）、
        //           pledge_count（质押笔数）、controlling_pledge_ratio（控股股东质押比例%）、
        //           risk_level（安全/低风险/中风险/高风险/极高风险）
        json!({
            "name": "get_stock_pledge_data",
            "description": "获取股权质押数据（大股东质押比例/质押股数/控股股东质押比例/风险等级），用于质押风险评估",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" }
                },
                "required": ["stock_code"]
            }
        }),
        // ── 算法工具 ──
        json!({
            "name": "compute_scoring",
            "description": "六维度技术评分（趋势/乖离/MACD/量能/RSI/支撑）+ 基本面修正 + 价值修正，返回100分制评分、买入信号、完整技术指标(ma5/ma20/bias_ma5/macd_dif/rsi14/boll_upper等)和最新价",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" },
                    "kline_json": { "type": "string", "description": "上游K线节点输出的JSON" }
                },
                "required": ["stock_code"]
            }
        }),
        // ── G1 跨市场数据接入：美股/港股/外汇/基准指数 ──
        json!({
            "name": "get_international_stock_quote",
            "description": "获取美股/港股实时行情（价格/涨跌/成交量/市值）。支持 AAPL/00700.HK/TSLA/BABA 等代码格式",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "国际股票代码：AAPL / TSLA / 00700 / 00700.HK / BABA.US" }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "get_international_stock_kline",
            "description": "获取美股/港股历史 K 线（开高低收/成交量）。支持 daily/weekly/monthly 周期",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "国际股票代码：AAPL / 00700 / TSLA.US" },
                    "period": { "type": "string", "description": "周期：daily/weekly/monthly", "default": "daily" },
                    "limit": { "type": "integer", "description": "K线数量（1-1000）", "default": 120 }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "get_benchmark_kline",
            "description": "获取基准指数 K 线（标普500/纳指/恒生/上证等），用于跨市场对比分析",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "benchmark_code": { "type": "string", "description": "基准指数代码：SPX/IXIC/DJI/HSI/HSCEI（国际）或 000001.SH/399001/399006/000300（A股）" },
                    "period": { "type": "string", "description": "周期：daily/weekly/monthly", "default": "daily" },
                    "limit": { "type": "integer", "description": "K线数量（1-1000）", "default": 120 }
                },
                "required": ["benchmark_code"]
            }
        }),
        json!({
            "name": "get_forex_kline",
            "description": "获取外汇 K 线（USD/CNY、HKD/CNY 等），用于跨市场汇率风险分析",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "pair": { "type": "string", "description": "外汇对：USD/CNY、HKD/CNY、EUR/USD、USD/JPY" },
                    "period": { "type": "string", "description": "周期：daily/weekly/monthly", "default": "daily" },
                    "limit": { "type": "integer", "description": "K线数量（1-1000）", "default": 120 }
                },
                "required": ["pair"]
            }
        }),
        json!({
            "name": "compute_valuation",
            "description": "DCF两阶段估值 + 格雷厄姆公式 + Piotroski F-Score(0-9) + 护城河量化(0-100)，返回内在价值和安全边际",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" },
                    "financials_json": { "type": "string", "description": "上游财务节点输出的JSON" }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "compute_portfolio_risk",
            "description": "计算单股风险画像：年化波动率/最大回撤/夏普比率/ROE/毛利率/负债率/营收增速/PE，输出 stockRiskProfile 供下游 portfolio-mgr 决策",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_codes": { "type": "string", "description": "逗号分隔的股票代码列表（工作流节点传入，取第一个为主标的）" },
                    "stock_code": { "type": "string", "description": "单个6位股票代码（LLM 直接调用时使用）" },
                    "weights": { "type": "string", "description": "逗号分隔的持仓权重(0-1)，不填则等权（可选）" }
                },
                "required": []
            }
        }),
        json!({
            "name": "run_quality_gate",
            "description": "LLM报告质量门控：占位检测、失败标记检测、必采项覆盖率检查，返回A-F质量评级",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "reports_json": { "type": "string", "description": "分析师报告JSON，格式: {expert_id: report_text}" }
                },
                "required": ["reports_json"]
            }
        }),
        // ── Serenity 瓶颈筛选工具集（V58 补全，对接 astock-data 已有 API）──
        // 历史问题：seed_serenity.rs 注册了 7 个 ToolDef schema 但无 Rust 实现，
        // 运行时 ToolResolver 三级匹配全部落空，t-baseline-*/t-signal-* 14 个 ToolNode
        // 全部失败，c-bottleneck-trend* CodeNode 因上游缺失也失败。
        // 修复：在 execute_mcp_tool 中补全 7 个 match 分支，对接 astock-data 已有 API。
        json!({
            "name": "compute_industry_position",
            "description": "行业竞争地位分析：拉取个股及同行业可比公司，计算毛利率/ROE/负债率/R&D强度行业排名、CapEx/折旧比",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "compute_bottleneck_signals",
            "description": "瓶颈信号计算：基于多期财报计算存货周转天数变化、毛利率同比趋势、CapEx/折旧比，识别供给瓶颈迹象",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "compute_attention_score",
            "description": "计算个股关注度评分 0-100，越低越冷门，验证低关注度因子（覆盖研报数+新闻热度+换手率+共识差）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "check_exit_signals",
            "description": "检查个股退出信号：技术替代新闻、毛利率趋势、产能过剩、新进入者、需求放缓。返回 overall_exit_urgency",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" },
                    "entry_price": { "type": "number", "description": "买入价（可选，用于计算止损触发）" },
                    "stop_loss_price": { "type": "number", "description": "止损价（可选）" }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "verify_catalysts",
            "description": "验证 Serenity 候选的催化剂是否兑现：基于近期新闻公告匹配催化剂描述",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" },
                    "catalyst_descriptions": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "催化剂描述列表"
                    }
                },
                "required": ["stock_code"]
            }
        }),
        json!({
            "name": "compute_serenity_performance",
            "description": "计算 Serenity 候选推荐后表现：相对推荐日的涨跌幅、相对大盘超额收益、持有天数",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" },
                    "recommend_date": { "type": "string", "description": "推荐日期 YYYY-MM-DD" }
                },
                "required": ["stock_code", "recommend_date"]
            }
        }),
        json!({
            "name": "optimize_attention_weights",
            "description": "基于历史样本调优关注度评分权重：输入样本（attention_score + 实际表现），输出建议权重",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "samples": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "attention_score": { "type": "number" },
                                "actual_return_pct": { "type": "number" }
                            }
                        },
                        "description": "历史样本列表"
                    }
                },
                "required": ["samples"]
            }
        }),
        // G3 产业链相关 MCP 工具（get_industry_chain_propagation /
        // map_news_to_cross_market_stocks）已于 P2-8 阶段迁至
        // `axagent_analysis_engine::mcp_tools`。本 crate 不再注册这两个工具，
        // 调用方需通过 `axagent_analysis_engine::mcp_tools::industry_chain_mcp_tools()`
        // 获取并合并到工具列表中。
        // ── G14 DojoSDK 工具集 ──────────────────────────────────────────
        json!({
            "name": "sector_precomputed_sector_alpha_factors_daily",
            "description": "DojoSDK: 行业 alpha 因子日频数据。返回一级行业（申万）alpha 因子序列，含 size/value/momentum/reversal/volatility/liquidity 6 类因子值。可选 date 范围与行业过滤。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "start_date": { "type": "string", "description": "起始日期 YYYY-MM-DD（默认近 30 日）" },
                    "end_date": { "type": "string", "description": "结束日期 YYYY-MM-DD（默认今日）" },
                    "industry": { "type": "string", "description": "可选行业过滤（如 '银行'、'半导体'）；不传 = 全行业" },
                    "factors": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "需要返回的因子列表，可选: size/value/momentum/reversal/volatility/liquidity；默认全部"
                    }
                }
            }
        }),
        json!({
            "name": "dojo_run_quant_backtest",
            "description": "DojoSDK: 运行量化策略回测。内置 5 套策略 (ma_cross/macd/rsi/boll/turtle)，返回完整 BacktestResult 含交易记录、净值曲线、Sharpe/MaxDD 等指标。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6 位股票代码" },
                    "strategy": { "type": "string", "enum": ["ma_cross","macd","rsi","boll","turtle"], "description": "内置策略名" },
                    "start_date": { "type": "string", "description": "回测起始日期 YYYY-MM-DD" },
                    "end_date": { "type": "string", "description": "回测结束日期 YYYY-MM-DD" },
                    "initial_capital": { "type": "number", "description": "初始资金（默认 100000）" },
                    "params": { "type": "object", "description": "策略参数（如 {fast:5, slow:20}），可选" }
                },
                "required": ["stock_code", "strategy", "start_date", "end_date"]
            }
        }),
        json!({
            "name": "dojo_get_skill_content",
            "description": "DojoSDK: 获取指定 SKILL 的完整内容（含 frontmatter + 正文）。优先走 SkillPromptCache 缓存；缓存未命中则扫描 skill_dirs。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "skill_name": { "type": "string", "description": "skill 名称（目录名，如 stock-pick / industry-chain-analysis / risk-management / market-mainline）" }
                },
                "required": ["skill_name"]
            }
        }),
        json!({
            "name": "dojo_list_skills",
            "description": "DojoSDK: 列出当前所有可用 SKILL（含内置 4 个 + 用户自定义），返回 [{name, description, version, source_kind}]。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "include_external": { "type": "boolean", "description": "是否包含外部目录（claude/trae/codebuddy 等），默认 true" }
                }
            }
        }),
        json!({
            "name": "dojo_get_paper_portfolio",
            "description": "DojoSDK: 获取模拟观察组合详情（含持仓 + 实时盈亏）。组合状态 active/closed/archived。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "portfolio_id": { "type": "string", "description": "组合 ID" }
                },
                "required": ["portfolio_id"]
            }
        }),
        json!({
            "name": "dojo_list_market_mainlines",
            "description": "DojoSDK: 列出最近 N 天市场主线（按强度降序）。可选 category 过滤。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "days": { "type": "integer", "description": "最近天数（默认 7）" },
                    "category": { "type": "string", "description": "主题大类过滤（可选）" }
                }
            }
        }),
        json!({
            "name": "dojo_create_plan",
            "description": "DojoSDK G19: 创建分层执行计划。基于目标拆分为多阶段（Phase）+ 多任务（Task），支持任务间依赖与角色分配。复用 HierarchicalPlanner。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "goal": { "type": "string", "description": "计划目标描述（如'分析半导体产业链投资机会'）" },
                    "phases": {
                        "type": "array",
                        "description": "阶段数组，每个阶段包含多个任务",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string", "description": "阶段名称" },
                                "description": { "type": "string", "description": "阶段描述" },
                                "dependencies": {
                                    "type": "array",
                                    "items": { "type": "string" },
                                    "description": "依赖的阶段 ID（可空，从 1 开始计数：1=第一Phase）"
                                },
                                "tasks": {
                                    "type": "array",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "description": { "type": "string", "description": "任务描述" },
                                            "action_type": {
                                                "type": "string",
                                                "description": "动作类型：agent/llm/tool/shell",
                                                "default": "agent"
                                            },
                                            "parameters": { "type": "object", "description": "任务参数（JSON）" },
                                            "dependencies": {
                                                "type": "array",
                                                "items": { "type": "string" },
                                                "description": "依赖任务 ID（同阶段内）"
                                            },
                                            "max_retries": { "type": "integer", "default": 3 },
                                            "assigned_role": {
                                                "type": "string",
                                                "description": "分配角色（analyst/implementer/reviewer）"
                                            }
                                        },
                                        "required": ["description", "action_type"]
                                    }
                                }
                            },
                            "required": ["name", "description", "tasks"]
                        }
                    }
                },
                "required": ["goal", "phases"]
            }
        }),
        json!({
            "name": "dojo_execute_plan",
            "description": "DojoSDK G19: 启动/继续执行已创建的计划。返回当前进度与下一批可执行任务。复用 HierarchicalPlanner。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "plan_id": { "type": "string", "description": "计划 ID（由 create_plan 返回）" },
                    "action": {
                        "type": "string",
                        "enum": ["start", "pause", "resume", "cancel", "progress", "next_tasks", "complete_task", "fail_task"],
                        "default": "start",
                        "description": "执行动作：start=开始执行, pause=暂停, resume=继续, cancel=取消, progress=查询进度, next_tasks=获取下一批可执行任务, complete_task=标记任务完成, fail_task=标记任务失败"
                    },
                    "task_id": { "type": "string", "description": "complete_task/fail_task 必填" },
                    "result": { "type": "object", "description": "complete_task 时附带的任务结果" },
                    "error": { "type": "string", "description": "fail_task 时的错误信息" }
                },
                "required": ["plan_id", "action"]
            }
        }),
        json!({
            "name": "dojo_revise_plan",
            "description": "DojoSDK G19: 修订计划（重规划）。支持 Retry/Skip/Insert/Remove/Reorder/AddPhase/ModifyTask 七种动作。复用 HierarchicalPlanner.replan。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "plan_id": { "type": "string", "description": "计划 ID" },
                    "reason": {
                        "type": "string",
                        "description": "重规划原因（StepFailed/ResourceConstraint/GoalChanged/NewDependencyDiscovered/ManualIntervention）"
                    },
                    "actions": {
                        "type": "array",
                        "description": "重规划动作数组",
                        "items": {
                            "type": "object",
                            "properties": {
                                "type": {
                                    "type": "string",
                                    "enum": ["Retry", "Skip", "Insert", "Remove", "Reorder", "AddPhase", "ModifyTask"]
                                },
                                "task_id": { "type": "string", "description": "Retry/Skip/Remove/Reorder/ModifyTask 必填" },
                                "phase_id": { "type": "string", "description": "Insert 必填" },
                                "modified_parameters": { "type": "object", "description": "Retry 时修改参数" },
                                "reason": { "type": "string", "description": "Skip/Remove 原因" },
                                "task": { "type": "object", "description": "Insert 时新任务定义" },
                                "position": { "type": "integer", "description": "Insert/Reorder/AddPhase 位置" },
                                "new_position": { "type": "integer", "description": "Reorder 新位置" },
                                "phase": { "type": "object", "description": "AddPhase 新阶段" },
                                "modifications": { "type": "object", "description": "ModifyTask 修改字段" }
                            },
                            "required": ["type"]
                        }
                    },
                    "rollback_to_version": {
                        "type": "integer",
                        "description": "可选：回滚到指定版本（不传则执行 actions）"
                    }
                },
                "required": ["plan_id", "reason"]
            }
        }),
    ]
}

pub async fn execute_mcp_tool(
    client: &crate::AStockClient,
    tool_name: &str,
    arguments: &serde_json::Value,
) -> Result<String, String> {
    // 辅助函数:兼容 LLM 传入数字或字符串类型的 stock_code
    // 修复(2026-07-22): GLM-5.2 偶尔传入 {"stock_code": 600887} (数字) 而非
    // {"stock_code": "600887"} (字符串),导致 as_str() 返回 None → 空字符串。
    let parse_code = |args: &serde_json::Value| -> String {
        // 修复(2026-07-22): GLM-5.2 偶传 {"stock_code": 600887} (数字)
        // 修复(2026-07-30): GLM-5.2 偶传 {"argument": 601398} (泛化键名)
        // 修复(2026-07-30 第2版): GLM-5.2 偶传 {"argument": {"stock_code": "601899"}} (嵌套对象)
        let from_key = |key: &str| -> String {
            match &args[key] {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                _ => String::new(),
            }
        };
        let code = from_key("stock_code");
        if !code.is_empty() {
            return code;
        }
        // 兼容 "argument" 为字符串/数字/嵌套对象三种形态
        match &args["argument"] {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Object(obj) => {
                // GLM-5.2 偶传 {"argument": {"stock_code": "601899"}}
                if let Some(v) = obj.get("stock_code") {
                    match v {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Number(n) => n.to_string(),
                        _ => String::new(),
                    }
                } else {
                    String::new()
                }
            },
            _ => String::new(),
        }
    };
    // 同上,兼容 keyword 的数字/字符串类型
    let parse_str = |args: &serde_json::Value, key: &str| -> String {
        match &args[key] {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            _ => String::new(),
        }
    };

    // G14: DojoSDK 工具集优先委托给已注册的 DojoSdkExecutor
    // （astock-data 不能直接依赖 quant/stock-analysis/tools，故走 trait 注入）
    if is_dojo_sdk_tool(tool_name) {
        return with_dojo_sdk_executor(tool_name, arguments).await;
    }

    // P0 修复(2026-07-22): 对需要 stock_code 的工具统一做空值预检，
    // 避免空字符串传给 vendor 后触发 6 vendor × 2 轮无效重试（浪费 ~3 分钟）。
    // 根因：Agent 节点 LLM 流式 tool_call arguments 反序列化失败时 stock_code 为空，
    // parse_code 返回空字符串 → to_em_secid("") → "0." → vendor 全部失败 → 重试。
    // 以下工具不需要 stock_code（用 keyword 或无参数），排除在预检之外。
    if !matches!(
        tool_name,
        "search_stock"
            | "search_news"
            | "get_market_dragon_tiger"
            | "get_hot_stocks"
            | "get_industry_ranking"
            | "get_cls_flash"
            | "get_north_bound_flow"
            | "get_index_quotes"
            | "compute_portfolio_risk"
            | "optimize_attention_weights"
            | "get_forex_kline"
            | "get_benchmark_kline"
            // G3 industry_chain 工具已迁至 axagent_analysis_engine::mcp_tools
            | "dojo_run_quant_backtest"
            | "dojo_get_skill_content"
            | "dojo_list_skills"
            | "dojo_get_paper_portfolio"
            | "dojo_list_market_mainlines"
            | "dojo_create_plan"
            | "dojo_execute_plan"
            | "dojo_revise_plan"
            | "sector_precomputed_sector_alpha_factors_daily"
    ) {
        let code = parse_code(arguments);
        if code.is_empty() {
            tracing::warn!(
                tool = tool_name,
                args = %arguments,
                "stock_code 为空（LLM 参数解析失败），快速失败避免无效重试"
            );
            return Err(format!(
                "工具 '{}' 缺少 stock_code 参数（LLM 参数解析失败，arguments={}）",
                tool_name, arguments
            ));
        }
    }

    match tool_name {
        "search_stock" => {
            let keyword = parse_str(arguments, "keyword");
            if keyword.trim().is_empty() {
                return Err(
                    "search_stock 缺少 keyword 参数，请传入完整中文名称或6位数字代码".to_string()
                );
            }
            // 拼音片段检测在 AStockClient::search_stock 底层统一处理
            // （覆盖 Tauri 命令 + MCP 工具两条路径，避免重复逻辑）
            let keyword = keyword.as_str();
            let results = client.search_stock(keyword).await.map_err(|e| e.to_string())?;
            serde_json::to_string(&results).map_err(|e| e.to_string())
        },
        "search_news" => {
            let keyword = parse_str(arguments, "keyword");
            let keyword = keyword.as_str();
            let limit = arguments["limit"].as_u64().unwrap_or(10) as u32;
            let results = client.search_news(keyword, limit).await.map_err(|e| e.to_string())?;
            serde_json::to_string(&results).map_err(|e| e.to_string())
        },
        "get_stock_quote" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            let quote = client.get_quote(code).await.map_err(|e| e.to_string())?;
            serde_json::to_string(&quote).map_err(|e| e.to_string())
        },
        "get_stock_kline" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            let period = arguments["period"].as_str().unwrap_or("daily");
            let limit = arguments["limit"].as_u64().unwrap_or(120).min(500) as u32;
            let klines = client.get_klines(code, period, limit).await.map_err(|e| e.to_string())?;
            serde_json::to_string(&klines).map_err(|e| e.to_string())
        },
        // ── G1 跨市场数据接入 ──
        "get_international_stock_quote" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            if code.is_empty() {
                return Err("get_international_stock_quote 缺少 stock_code 参数".to_string());
            }
            // 国际代码直接走 international vendor 路径
            let quote = if crate::is_international_code(code) {
                client.get_international_quote(code).await.map_err(|e| e.to_string())?
            } else {
                // 兼容：传入 A 股代码时走默认路由
                client.get_quote(code).await.map_err(|e| e.to_string())?
            };
            serde_json::to_string(&quote).map_err(|e| e.to_string())
        },
        "get_international_stock_kline" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            if code.is_empty() {
                return Err("get_international_stock_kline 缺少 stock_code 参数".to_string());
            }
            let period = arguments["period"].as_str().unwrap_or("daily");
            let limit = arguments["limit"].as_u64().unwrap_or(120).min(1000) as u32;
            let klines = if crate::is_international_code(code) {
                client
                    .get_international_klines(code, period, limit, None)
                    .await
                    .map_err(|e| e.to_string())?
            } else {
                client.get_klines(code, period, limit).await.map_err(|e| e.to_string())?
            };
            serde_json::to_string(&klines).map_err(|e| e.to_string())
        },
        "get_benchmark_kline" => {
            let benchmark = parse_str(arguments, "benchmark_code");
            if benchmark.trim().is_empty() {
                return Err("get_benchmark_kline 缺少 benchmark_code 参数".to_string());
            }
            let period = arguments["period"].as_str().unwrap_or("daily");
            let limit = arguments["limit"].as_u64().unwrap_or(120).min(1000) as u32;
            let klines = client
                .get_benchmark_klines(&benchmark, period, limit)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_string(&klines).map_err(|e| e.to_string())
        },
        "get_forex_kline" => {
            let pair = parse_str(arguments, "pair");
            if pair.trim().is_empty() {
                return Err("get_forex_kline 缺少 pair 参数（如 USD/CNY）".to_string());
            }
            let period = arguments["period"].as_str().unwrap_or("daily");
            let limit = arguments["limit"].as_u64().unwrap_or(120).min(1000) as u32;
            let klines =
                client.get_forex_klines(&pair, period, limit).await.map_err(|e| e.to_string())?;
            serde_json::to_string(&klines).map_err(|e| e.to_string())
        },
        "get_stock_financials" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            let financials = client.get_financials(code).await.map_err(|e| e.to_string())?;
            serde_json::to_string(&financials).map_err(|e| e.to_string())
        },
        "get_fundamentals_report_markdown" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            let quote = client.get_quote(code).await.map_err(|e| e.to_string())?;
            let financials = client.get_financials(code).await.map_err(|e| e.to_string())?;
            let report = crate::fundamentals_report::FundamentalsAnalyzer::generate(
                code,
                &quote,
                &financials,
            );
            Ok(report.to_markdown())
        },
        "get_stock_news" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            let limit = arguments["limit"].as_u64().unwrap_or(30).min(100) as u32;
            let news = client.get_news(code, limit).await.map_err(|e| e.to_string())?;
            serde_json::to_string(&news).map_err(|e| e.to_string())
        },
        "get_stock_policy_news" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            let limit = arguments["limit"].as_u64().unwrap_or(30).min(100) as u32;
            let news = client.get_policy_news(code, limit).await.map_err(|e| e.to_string())?;
            serde_json::to_string(&news).map_err(|e| e.to_string())
        },
        "get_stock_money_flow" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            let flow = client.get_money_flow(code).await.map_err(|e| e.to_string())?;
            serde_json::to_string(&flow).map_err(|e| e.to_string())
        },
        "get_social_sentiment" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            let sentiment = client.get_social_sentiment(code).await.map_err(|e| e.to_string())?;
            serde_json::to_string(&sentiment).map_err(|e| e.to_string())
        },
        "get_stock_dragon_tiger" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            let dt = client.get_dragon_tiger(code).await.map_err(|e| e.to_string())?;
            serde_json::to_string(&dt).map_err(|e| e.to_string())
        },
        "get_stock_margin_data" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            let margin = client.get_margin_data(code).await.map_err(|e| e.to_string())?;
            serde_json::to_string(&margin).map_err(|e| e.to_string())
        },
        "get_stock_sector_info" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            let sector = client.get_sector_info(code).await.map_err(|e| e.to_string())?;
            serde_json::to_string(&sector).map_err(|e| e.to_string())
        },
        "get_stock_north_bound" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            let nb = client.get_north_bound_holding(code).await.map_err(|e| e.to_string())?;
            serde_json::to_string(&nb).map_err(|e| e.to_string())
        },
        "get_stock_lockup" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            let lockup = client.get_lockup_schedule(code).await.map_err(|e| e.to_string())?;
            serde_json::to_string(&lockup).map_err(|e| e.to_string())
        },
        "get_stock_lockup_bundle" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            let bundle = client.get_lockup_bundle(code).await.map_err(|e| e.to_string())?;
            serde_json::to_string(&bundle).map_err(|e| e.to_string())
        },
        "get_stock_shareholder_trades" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            let trades = client.get_shareholder_trades(code).await.map_err(|e| e.to_string())?;
            serde_json::to_string(&trades).map_err(|e| e.to_string())
        },
        "get_stock_dividend_records" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            let dividends = client.get_dividend_records(code).await.map_err(|e| e.to_string())?;
            serde_json::to_string(&dividends).map_err(|e| e.to_string())
        },
        "get_stock_research_reports" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            let reports = client.get_research_reports(code).await.map_err(|e| e.to_string())?;
            serde_json::to_string(&reports).map_err(|e| e.to_string())
        },
        "get_stock_consensus_eps" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            let eps = client.get_consensus_eps(code).await.map_err(|e| e.to_string())?;
            serde_json::to_string(&eps).map_err(|e| e.to_string())
        },
        "get_stock_concept_blocks" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            let blocks = client.get_concept_blocks(code).await.map_err(|e| e.to_string())?;
            serde_json::to_string(&blocks).map_err(|e| e.to_string())
        },
        "get_stock_announcements" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            let anns = client.get_announcements(code).await.map_err(|e| e.to_string())?;
            serde_json::to_string(&anns).map_err(|e| e.to_string())
        },
        "get_stock_block_trades" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            let bt = client.get_block_trades(code).await.map_err(|e| e.to_string())?;
            serde_json::to_string(&bt).map_err(|e| e.to_string())
        },
        "get_stock_institutional_visits" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            let visits = client.get_institutional_visits(code).await.map_err(|e| e.to_string())?;
            serde_json::to_string(&visits).map_err(|e| e.to_string())
        },
        "get_market_dragon_tiger" => {
            let dt = client.get_market_dragon_tiger().await.map_err(|e| e.to_string())?;
            serde_json::to_string(&dt).map_err(|e| e.to_string())
        },
        "get_hot_stocks" => {
            let hot = client.get_hot_stocks().await.map_err(|e| e.to_string())?;
            serde_json::to_string(&hot).map_err(|e| e.to_string())
        },
        "get_industry_ranking" => {
            let ranking = client.get_industry_ranking().await.map_err(|e| e.to_string())?;
            serde_json::to_string(&ranking).map_err(|e| e.to_string())
        },
        "get_cls_flash" => {
            let flash = client.get_cls_flash().await.map_err(|e| e.to_string())?;
            serde_json::to_string(&flash).map_err(|e| e.to_string())
        },
        "get_north_bound_flow" => {
            let flow = client.get_north_bound_flow().await.map_err(|e| e.to_string())?;
            serde_json::to_string(&flow).map_err(|e| e.to_string())
        },
        "get_index_quotes" => {
            let idx = client.get_index_quotes().await.map_err(|e| e.to_string())?;
            serde_json::to_string(&idx).map_err(|e| e.to_string())
        },
        "get_stock_peers" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            let peers = client.get_peers(code).await.map_err(|e| e.to_string())?;
            serde_json::to_string(&peers).map_err(|e| e.to_string())
        },
        "get_stock_option_pcr" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            let pcr = client.get_option_pcr(code).await.map_err(|e| e.to_string())?;
            serde_json::to_string(&pcr).map_err(|e| e.to_string())
        },
        // #4: 股权质押数据 — LLM 可先调用本工具拿到 pledge_pct,
        // 再调用 detect_pledge_risk (tools/finance.rs) 做阈值判断。
        "get_stock_pledge_data" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            let pledge = client.get_pledge_data(code).await.map_err(|e| e.to_string())?;
            serde_json::to_string(&pledge).map_err(|e| e.to_string())
        },
        // ── 算法工具：compute_scoring / compute_valuation / compute_portfolio_risk ──
        // 历史问题：工具列表（stock_mcp_tools）声明了这些算法工具，但 dispatch_tool
        // 的 match 中没有对应分支，LLM 调用时走到 `_ => Unknown MCP tool` 分支失败。
        // V57 修复：补全三个算法工具的分发，复用 astock-data 内的 ScoringEngine 等模块，
        // 避免重复实现（铁律 4）。
        "compute_scoring" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            if code.is_empty() {
                return Err("compute_scoring 缺少 stock_code 参数".to_string());
            }
            // 允许调用方传入 kline_json（避免重复拉取）；若未提供则现场拉取 120 日 K 线
            let klines = if let Some(kj) = arguments["kline_json"].as_str() {
                serde_json::from_str::<Vec<crate::types::KLine>>(kj)
                    .map_err(|e| format!("kline_json 解析失败: {e}"))?
            } else {
                client.get_klines(code, "daily", 120).await.map_err(|e| e.to_string())?
            };
            let ind = crate::indicators::compute_indicators(code, &klines);
            let latest_price = klines.last().map(|k| k.close).unwrap_or(0.0);
            let score = crate::scoring::ScoringEngine::score(&ind, latest_price, None);
            // #7 修复(2026-07-22): 原实现只返回 ObjectiveScore 评分结构,
            // 缺少 totalScore/currentPrice/indicators/factor_backtest 字段,
            // 导致下游 input_mapping 引用(t-scoring.result.indicators.rsi14 等)全部为 null,
            // LLM 报告中 MA5/MA20/bias_ma5 等技术指标缺失。
            //
            // 修复: 用 json! 构造扩展返回结构,既保留原 ObjectiveScore 字段(向后兼容),
            // 又追加 totalScore(别名)/currentPrice/indicators/factor_backtest(占位)。
            let score_json = serde_json::to_value(&score).map_err(|e| e.to_string())?;
            let ind_json = serde_json::to_value(&ind).map_err(|e| e.to_string())?;
            // P0 根因修复(2026-07-22): 返回 kline_json 供下游 trader 节点通过 input_mapping
            // 引用，避免 trader 重新调用 get_stock_kline（原设计导致 LLM 生成空 stock_code
            // 的 tool_call，触发 6 vendor × 2 轮无效重试，浪费 3.4 分钟）。
            // kline_json 是 120 根日 K 线的 JSON 数组，trader 可直接传给 compute_atr /
            // compute_kelly / compute_mc 等纯计算工具。
            let kline_json = serde_json::to_value(&klines).map_err(|e| e.to_string())?;
            let result = serde_json::json!({
                // ── 原 ObjectiveScore 字段(flatten 等价,向后兼容) ──
                "total": score_json["total"],
                "trendScore": score_json["trendScore"],
                "deviationScore": score_json["deviationScore"],
                "macdScore": score_json["macdScore"],
                "volumeScore": score_json["volumeScore"],
                "rsiScore": score_json["rsiScore"],
                "supportScore": score_json["supportScore"],
                "bollScore": score_json["bollScore"],
                "fundamentalAdjustment": score_json["fundamentalAdjustment"],
                "signal": score_json["signal"],
                "signalCode": score_json["signalCode"],
                // ── #7 新增: 别名 + 原始指标 + 占位字段 ──
                "totalScore": score_json["total"], // 别名,供 input_mapping 引用
                "currentPrice": latest_price,       // 最新收盘价
                "indicators": ind_json,             // 完整技术指标(ma5/ma20/bias_ma5/macd_dif/rsi14/boll_upper 等)
                // kline_json: 120 根日 K 线原始数据，供 trader 节点的 ATR/Kelly/MC 工具使用
                "kline_json": kline_json,
                // factor_backtest 占位: 因子回测引擎未实现,下游 portfolio-mgr.rhai
                // 会 fallback 到等权,不会因 null 报错。
                "factor_backtest": {
                    "factors": serde_json::json!({}),
                    "note": "factor backtest engine not implemented, using equal weights fallback"
                }
            });
            serde_json::to_string(&result).map_err(|e| e.to_string())
        },
        "compute_valuation" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            if code.is_empty() {
                return Err("compute_valuation 缺少 stock_code 参数".to_string());
            }
            // 尝试解析可选的估值配置参数。
            // 优先级：`valuation_config`(object) > 扁平参数 > 模块常量默认。
            // C2 路径 Z(2026-09-12)：模板侧 `input_mapping` 无法构造嵌套 object
            // （见 `from_flat_arguments` 注释），故设置面板的 `value_dcf_*` 变量
            // 经由扁平参数 `dcf_growth_rate` / `dcf_perpetual_rate` /
            // `dcf_discount_rate`（**百分数**口径）承接。
            let valuation_config = arguments
                .get("valuation_config")
                .and_then(|v| serde_json::from_value::<ValuationConfig>(v.clone()).ok())
                .or_else(|| ValuationConfig::from_flat_arguments(arguments));

            // 估值需要行情（PE/PB/总市值）和财务数据
            let quote = client.get_quote(code).await.map_err(|e| e.to_string())?;
            let financials = client.get_financials(code).await.map_err(|e| e.to_string())?;
            let current_price = quote.price;
            let pe = quote.pe;
            let pb = quote.pb;
            let total_mv = quote.total_mv;
            // P0 修复(2026-09-11): `Quote.total_mv` 的权威单位是「元」，不是「亿元」。
            // 判据: ① 本项目内另一个消费方 fundamentals_report.rs 直接 mv/rev 求 PS、
            //          fcf/mv 求 FCF 收益率，只有按元才成立；
            //       ② vendors/tencent.rs 显式把原始字段 ×1e8 归一到「元」，
            //          browser_eastmoney.rs 对 f116 注明「总市值，不乘 1e8」；
            //       ③ DB 实证 total_mv=1.76446e11 对应 price=620.88 元/股 →
            //          股本 2.84e8 股（合理），若按亿元则市值 1.76e19 元（荒谬）。
            // 前两版实现各错 1e8 倍: 原式 mv/price/1e8（股本偏小 1e8 → DCF 输出
            // 「亿元/股」级垃圾），V-fix 改为 mv*1e8/price（股本偏大 1e8 →
            // fcf_per_share 偏小 1e8 → DCF 三档恒为 0 → upsidePct 恒 -100 →
            // f5 估值因子权重 0.21 × 信号 -1.0 形成恒定 -7.9pt 拖累，全部样本同值、
            // 因子方差为 0 = 零信息量）。
            // 正确: 总股本(股) = 总市值(元) / 现价(元/股)
            let total_shares = if current_price > 0.0 {
                total_mv.map(|mv| mv / current_price)
            } else {
                None
            };

            // ── Piotroski F-Score (0-9) ──
            let f_score = compute_f_score(&financials);
            let f_score_level = match f_score {
                7..=9 => "优秀(7-9)",
                5..=6 => "良好(5-6)",
                3..=4 => "一般(3-4)",
                _ => "弱(0-2)",
            };

            // ── 护城河量化评分 (0-100) ──
            let (moat_score, moat_level) = compute_moat_score(&financials, pe, pb);

            // ── DCF 两阶段估值 ──
            // V74(2026-09-10): 原实现 FCF≤0 / 股本缺失时返回 (0,0,0)，「无法估值」被
            // 编码成「估值为0」，value-investor 据此输出「理想买入价0元」、f5 吃到
            // 0% 上行空间——全是退化垃圾。改为 Option + 归一化 FCF fallback：
            // 当期亏损（周期底部）用近 5 年报正净利均值×0.90 锚定，持续亏损才返回 None。
            let (dcf_tiers, dcf_note, dcf_assumptions) =
                compute_dcf(&financials, total_shares, current_price, valuation_config.as_ref());
            let (dcf_low, dcf_mid, dcf_high) = match dcf_tiers {
                Some((l, m, h)) => (Some(l), Some(m), Some(h)),
                None => (None, None, None),
            };

            // ── 格雷厄姆内在价值（先计算，作为 DCF fallback） ──
            // V74: EPS≤0 时同样走归一化 EPS fallback，均不可用才返回 None
            let graham_value =
                compute_graham_value(&financials, current_price, valuation_config.as_ref());

            // ── 估值输出量程守卫（下界 P0 修复 2026-09-11 晚；上界补齐 2026-09-12）──
            // 原判据有两处都无防护力：
            //   · `dcf_mid.is_some()`   → 值存在即算可用，退化值照样通过；
            //   · `mid > f64::EPSILON`  → `f64::EPSILON ≈ 2.22e-16`，而量纲错产生的
            //     退化值在 1e-7 量级，**远大于它**，守卫形同虚设。
            // DB 实证：`total_mv` 单位错时 mid ≈ 1.7e-7（现价 620.88 元的 ~1e-10 倍）
            //   → `dcf.upsidePct` 恒 -100、`mos_pct` 输出 -5.7e10%（垃圾值），
            //   且 `available` 谎报 true —— 与 value-investor.md / 模板 prompt 的契约
            //   （「available=false 或 upsidePct=null 时填 null」）直接冲突。
            // 判据取「现价的 1%」：退化值（~1e-10 × 现价）与真实「极端高估」
            //   （最低也在 1% 量级）之间有 3 个数量级安全边际，不会误伤真实结论。
            let iv_floor = current_price * 0.01;
            // 上界补齐（2026-09-12）：原守卫只挡「太小」，不挡「太大」，而两个方向同源。
            // DB 实证 002837@2026-09-09：dcf.low/mid/high = 2.667e8 / 3.861e8 / 6.020e8
            //   （现价 62.43 元）→ `dcf.upsidePct` = **+618,509,774.8%**，
            //   被 f5 的 `pm_saturate(x, 40)` 饱和成 **+1.0** ⇒ 用伪造数据把估值因子
            //   推成「极度低估」。**这比负向退化更危险 —— 它会制造看多信号**。
            // 根因与 mid→0 同一个：`total_shares` 量纲错 1e8，只是那一版是偏小
            //   （`mv/price/1e8` → 每股 FCF 偏大 1e8），09-09 样本走的是它。
            // 判据取「现价的 100 倍」：A 股最极端的真实低估也在 10 倍量级，留一个数量级余量；
            //   而与爆炸值（6e6 × 现价）之间仍有 4 个数量级安全边际。
            // 同一把尺子也适用于格雷厄姆（见下方 graham_usable），否则两个消费者口径不一致。
            let iv_ceil = current_price * 100.0;
            let dcf_usable =
                current_price > 0.0 && dcf_mid.is_some_and(|m| m > iv_floor && m < iv_ceil);
            // 退化输出一律按「不可用」处理（V74 契约：输出 null 而非 0），
            // 避免下游把 ≈0 当成「内在价值 0 元」的真实估值。
            // 此处遮蔽 dcf_low/mid/high 后，下游所有 `dcf_mid.is_none()` 判断
            // （valuation_unavailable / mos_level / available）自动收敛为正确语义。
            let (dcf_low, dcf_mid, dcf_high) = if dcf_usable {
                (dcf_low, dcf_mid, dcf_high)
            } else {
                (None, None, None)
            };

            // ── 格雷厄姆输出同一把尺子（2026-09-12 补齐）──
            // 原实现**完全没有守卫**：`graham.upsidePct` 只判 `Some(g) && current_price > 0`，
            //   既不挡下界也不挡上界。而 `mos_pct` 的 graham 分支早已用 `iv_floor`
            //   ⇒ 同一个值在两条消费路径上被两套口径判「可用/不可用」，属于一致性缺陷。
            // 格雷厄姆走 EPS 链（不经过 `total_shares`），所以不受量纲错影响；
            //   但 EPS 近零时 g 会落在 1e-7 量级 → upsidePct 仍是 −100 级垃圾；
            //   EPS 爆炸时同理产出 +1e8 级伪造看多。两侧都要挡。
            // 遮蔽后：`graham_intrinsic_value` 输出 null、`graham.upsidePct` 输出 null、
            //   下游 `valuation_unavailable` / mos_pct fallback 自动收敛为正确语义。
            let graham_usable =
                current_price > 0.0 && graham_value.is_some_and(|g| g > iv_floor && g < iv_ceil);
            let graham_value = if graham_usable { graham_value } else { None };

            // ── 安全边际：优先使用 DCF，不可用时 fallback 到格雷厄姆，均不可用为 None ──
            // P0 修复(2026-09-11): 原实现未对分母（内在价值）做零值防护。当
            // dcf_mid = Some(0.0) 时 (mid - price)/mid = -inf，DB 实证输出
            // 「安全边际 -57255189717%(无（高估风险）)」级垃圾值，并污染 value_signal。
            // 注意: mos_pct 的分母按教科书定义为「内在价值」（折价率口径），
            // 与 dcf.upsidePct 的「现价」分母（上行空间口径）是两个不同指标，不可互换；
            // 此处仅加零值防护，不改口径。
            let mos_pct: Option<f64> = match (dcf_mid, graham_value) {
                // 守卫改用 iv_floor（相对阈值）：`f64::EPSILON` 挡不住 1e-7 级退化值。
                // 上界 iv_ceil 为 2026-09-12 补齐 —— 分母超大时 mos → +100，
                // 会被读成「充足安全边际」，同样是伪造的看多信号（实证见 iv_ceil 注释）。
                // 注：dcf_mid / graham_value 已在上方遮蔽过，此处守卫是双保险（防将来挪动代码顺序）。
                (Some(mid), _) if mid > iv_floor && mid < iv_ceil => {
                    Some(((mid - current_price) / mid) * 100.0)
                },
                // 格雷厄姆分支同理：它不经 `total_shares`（不受该单位 bug 影响），
                // 但 EPS 近零时同样会输出同量级垃圾，需同一把尺子
                (None, Some(g)) if g > iv_floor && g < iv_ceil => {
                    Some(((g - current_price) / g) * 100.0)
                },
                _ => None,
            };
            let mos_level: String = match mos_pct {
                Some(mos) => {
                    let base = if mos > 30.0 {
                        "充足"
                    } else if mos > 15.0 {
                        "适中"
                    } else if mos > 0.0 {
                        "不足"
                    } else {
                        "无（高估风险）"
                    };
                    if dcf_mid.is_none() {
                        format!("{}(格雷厄姆)", base)
                    } else {
                        base.to_string()
                    }
                },
                None => "无法计算（DCF与格雷厄姆均不适用）".to_string(),
            };

            // ── 所有者收益率 ──
            // P0 修复(2026-09-11): 与上方 total_shares 同源错误。原注释按「total_mv 为亿元」
            // 再乘 1e8，而 total_mv 实为「元」→ 分母被放大 1e8 倍 →
            // oe_yield 恒为 0.0%（DB 实证：全部样本 owner_earnings_yield_pct = 0.0）。
            // 正确: oe(元) / mv(元) × 100
            let oe_yield =
                if let (Some(mv), Some(oe)) = (total_mv, compute_owner_earnings(&financials)) {
                    if mv > 0.0 {
                        (oe / mv) * 100.0
                    } else {
                        0.0
                    }
                } else {
                    0.0
                };

            // ── 综合估值判断 ──
            // V74: DCF 与格雷厄姆均不可用时输出「无法估值」——无估值锚 ≠ 高估，
            // 旧逻辑会因 score 兜底恒为「高估」污染下游 value-investor 判断
            let valuation_unavailable = dcf_mid.is_none() && graham_value.is_none();
            let value_signal: String = if valuation_unavailable {
                "无法估值".to_string()
            } else {
                let mut score = 0u32;
                match mos_pct {
                    Some(p) if p > 20.0 => score += 30,
                    Some(p) if p > 10.0 => score += 20,
                    Some(p) if p > 0.0 => score += 10,
                    _ => {},
                }
                score += f_score.min(9) * 5;
                score += moat_score.min(100) / 5;
                if oe_yield > 5.0 {
                    score += 20;
                } else if oe_yield > 3.0 {
                    score += 10;
                }
                match score {
                    60.. => "低估",
                    45.. => "合理偏低",
                    30.. => "合理",
                    15.. => "偏高",
                    _ => "高估",
                }
                .to_string()
            };

            // V74: 估值不可用时输出 null 而非 0。
            // 下游 portfolio-mgr.rhai 的 present() 对 null（Rhai unit）返回 false，
            // f5 估值因子自动降权为 0（估值维度不参与），data-quality 会正确上报
            // 「估值上行空间缺失」，不会把 0 当成真实估值。
            let num_or_null = |v: Option<f64>, round: fn(f64) -> f64| -> serde_json::Value {
                match v {
                    Some(x) => json!(round(x)),
                    None => serde_json::Value::Null,
                }
            };

            let summary = if valuation_unavailable {
                format!(
                    "内在价值: 不可用({}) | 格雷厄姆值: 不可用 | 安全边际: {} | F-Score={}/9({}) | 护城河{}/100({}) | OE收益率{:.1}% | 综合判断:{}",
                    dcf_note, mos_level, f_score, f_score_level, moat_score, moat_level, oe_yield, value_signal
                )
            } else {
                format!(
                    "内在价值(DCF中性)≈{}元 | 格雷厄姆值≈{}元 | 安全边际{}%({}) | F-Score={}/9({}) | 护城河{}/100({}) | OE收益率{:.1}% | 综合判断:{}",
                    dcf_mid.map(|v| format!("{:.2}", v)).unwrap_or_else(|| "不可用".into()),
                    graham_value.map(|v| format!("{:.2}", v)).unwrap_or_else(|| "不可用".into()),
                    mos_pct.map(|p| format!("{:.0}", p)).unwrap_or_else(|| "无法计算".into()),
                    mos_level, f_score, f_score_level, moat_score, moat_level, oe_yield, value_signal
                )
            };

            let result = json!({
                "stock_code": code,
                "current_price": current_price,
                "pe": pe,
                "pb": pb,
                "total_mv": total_mv,
                "dcf_valuation": {
                    "low": num_or_null(dcf_low, round2),
                    "mid": num_or_null(dcf_mid, round2),
                    "high": num_or_null(dcf_high, round2),
                    "available": dcf_usable,
                    "note": dcf_note,
                    // 2026-09-12：实际生效参数快照。修复前只落三档数值，导致
                    // 「mid=2.18 是怎么算出来的」必须靠反解，而 3 个方程解 5 个
                    // 未知量 ⇒ 欠定（多组参数同时命中）。落参数后对账即唯一解。
                    "assumptions": dcf_assumptions,
                },
                "graham_intrinsic_value": num_or_null(graham_value, round2),
                "margin_of_safety": {
                    "pct": num_or_null(mos_pct, round1),
                    "level": mos_level,
                },
                "piotroski_f_score": {
                    "score": f_score,
                    "max": 9,
                    "level": f_score_level,
                },
                "moat": {
                    "score": moat_score,
                    "max": 100,
                    "level": moat_level,
                    // 别名: 供 input_mapping 引用(与旧模板 moat.label 对齐)
                    "label": moat_level,
                },
                // ── camelCase 别名块(2026-09-09) ──
                // 模板 input_mapping 引用 dcf.upsidePct / graham.upsidePct /
                // fScore.score（上行空间百分比语义），原实现缺这些
                // 键导致 f5 估值因子恒缺失。upsidePct = (估值-现价)/现价*100。
                // V74: 估值不可用时为 null（下游 present() 视为缺失），不再输出 0。
                // 2026-09-12: 「不可用」判据 = 超出 [iv_floor, iv_ceil] 量程（上方已遮蔽）。
                //   f5 侧口径是 `pm_saturate(x, 40)`（rhai :578，不是 ±30% 硬夹），此处注释同步。
                // ⚠️ 两个 upsetPct 都不需要各自再写守卫 —— 依赖的是上方遮蔽；
                //   若将来挪动遮蔽位置，此处会静默退化（恒 -100 / +1e8 级），务必同步复核。
                "dcf": {
                    "low": num_or_null(dcf_low, round2),
                    "mid": num_or_null(dcf_mid, round2),
                    "high": num_or_null(dcf_high, round2),
                    "available": dcf_usable,
                    "note": dcf_note,
                    // 与 `dcf_valuation.assumptions` 同源同值（同一份快照）——
                    // 两处都放是为了让 `input_mapping` 任一引用路径都能取到。
                    "assumptions": dcf_assumptions,
                    "upsidePct": match dcf_mid {
                        Some(mid) if current_price > 0.0 => json!(round1((mid - current_price) / current_price * 100.0)),
                        _ => serde_json::Value::Null,
                    },
                },
                "graham": {
                    "intrinsicValue": num_or_null(graham_value, round2),
                    "upsidePct": match graham_value {
                        Some(g) if current_price > 0.0 => json!(round1((g - current_price) / current_price * 100.0)),
                        _ => serde_json::Value::Null,
                    },
                },
                "fScore": {
                    "score": f_score,
                    "level": f_score_level,
                },
                "owner_earnings_yield_pct": round1(oe_yield),
                "value_signal": value_signal,
                "summary": summary,
            });
            serde_json::to_string(&result).map_err(|e| e.to_string())
        },
        "compute_portfolio_risk" => {
            // 修复(2026-07-21):
            // 1) 参数名兼容: 节点传 `stock_codes`(逗号分隔), LLM 直接调用传 `stock_code`(单数)
            // 2) 输出结构对齐 portfolio-mgr.rhai 期望的 stockRiskProfile 字段
            //    (annualizedVolatilityPct/maxDrawdownPct/sharpeRatio/roeTTMPct/
            //     grossMarginPct/debtRatioPct/revenueGrowthYoYPct/peTTM)
            // 3) 用真实 K 线计算波动率/回撤/夏普, 用财报提取基本面指标
            let primary_code = arguments["stock_codes"]
                .as_str()
                .and_then(|s| s.split(',').next())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .or_else(|| arguments["stock_code"].as_str().map(str::trim))
                .ok_or_else(|| {
                    "compute_portfolio_risk 缺少 stock_codes/stock_code 参数".to_string()
                })?;

            // 拉取 60 日前复权 K 线计算量化风险指标
            let klines = client
                .get_klines_with_adj(
                    primary_code,
                    "daily",
                    60,
                    Some(crate::types::AdjType::Forward),
                )
                .await
                .map_err(|e| e.to_string())?;

            let (ann_vol_pct, max_dd_pct, sharpe) = if klines.len() >= 2 {
                let closes: Vec<f64> = klines.iter().map(|k| k.close).collect();
                // 日收益率序列
                let returns: Vec<f64> = closes
                    .windows(2)
                    .map(|w| {
                        if w[0] > 0.0 {
                            (w[1] - w[0]) / w[0]
                        } else {
                            0.0
                        }
                    })
                    .collect();
                // P3-C8: 夏普比率统一走 harness 实现（样本方差 n-1，A 股 244 天年化）。
                // 修复历史 bug: 原实现误用总体方差（n 分母），且 252/244 混用导致
                // 与 stock-analysis/risk.rs 的 Sharpe 结果分叉。
                // 保留 rf=3% 作为 A 股长期无风险利率近似（10 年期国债中枢）。
                let rf_daily = 0.03 / axagent_harness::indicators::A_SHARE_TRADING_DAYS_PER_YEAR;
                let sharpe = axagent_harness::indicators::sharpe_ratio_with_annualization(
                    &returns,
                    rf_daily,
                    axagent_harness::indicators::A_SHARE_TRADING_DAYS_PER_YEAR,
                );
                // 年化波动率: 复用 harness stddev_sample 保持算法一致（样本方差 n-1）
                let mean = returns.iter().sum::<f64>() / returns.len() as f64;
                let std = axagent_harness::indicators::stddev_sample(&returns, mean);
                let ann_vol =
                    std * axagent_harness::indicators::A_SHARE_TRADING_DAYS_PER_YEAR.sqrt() * 100.0;
                // 最大回撤
                let mut peak = closes[0];
                let mut max_dd = 0.0_f64;
                for &p in &closes {
                    if p > peak {
                        peak = p;
                    }
                    if peak > 0.0 {
                        let dd = (peak - p) / peak;
                        if dd > max_dd {
                            max_dd = dd;
                        }
                    }
                }
                let max_dd_pct = max_dd * 100.0;
                (
                    (ann_vol * 10.0).round() / 10.0,
                    (max_dd_pct * 10.0).round() / 10.0,
                    (sharpe * 1000.0).round() / 1000.0,
                )
            } else {
                (0.0, 0.0, 0.0)
            };

            // 拉取财报提取基本面指标(取最新一条)
            let financials =
                client.get_financials(primary_code).await.map_err(|e| e.to_string())?;
            let fin = financials.first();
            // 2026-09-14 修复：原实现是 `fin.and_then(|f| f.roe)` —— 直接取**最新一期**的
            // `roe`。中报口径下那是「年内累计值」（半年），却以 `roeTTMPct` 之名输出，
            // 与本段 `peTTM`（年化 EPS 口径）自相矛盾。
            // 601166 实证：4.8%（半年）被 risk-agg/neu/con + research-mgr 四份报告
            // 当作「严重偏离行业均值 10-12%」的核心看空论据，年化后实为 ~9.8%。
            let roe_ttm_pct = annualized_roe(&financials).map(|v| (v * 10.0).round() / 10.0);
            let gross_margin_pct =
                fin.and_then(|f| f.gross_margin).map(|v| (v * 10.0).round() / 10.0);
            let debt_ratio_pct = fin.and_then(|f| f.debt_ratio).map(|v| (v * 10.0).round() / 10.0);
            let revenue_growth_yoy_pct =
                fin.and_then(|f| f.revenue_yoy).map(|v| (v * 10.0).round() / 10.0);

            // 拉取行情拿 PE/PB
            let quote = client.get_quote(primary_code).await.map_err(|e| e.to_string())?;
            let pe_ttm = quote.pe;

            let result = json!({
                "stock_code": primary_code,
                "stockRiskProfile": {
                    "annualizedVolatilityPct": ann_vol_pct,
                    "maxDrawdownPct": max_dd_pct,
                    "sharpeRatio": sharpe,
                    "roeTTMPct": roe_ttm_pct,
                    "grossMarginPct": gross_margin_pct,
                    "debtRatioPct": debt_ratio_pct,
                    "revenueGrowthYoYPct": revenue_growth_yoy_pct,
                    "peTTM": pe_ttm,
                },
                "risk_note": "基于60日前复权K线计算波动率/回撤/夏普, 基本面指标取最新财报",
            });
            serde_json::to_string(&result).map_err(|e| e.to_string())
        },
        // ── Serenity 瓶颈筛选工具集（V58 补全）──
        // 输出契约与 bottleneck-calc.rhai 期望字段对齐：
        //   competitive_position.gross_margin_pct / roe_pct / debt_ratio_pct / rnd_intensity
        //   capacity_indicators.signal / sector
        "compute_industry_position" => {
            let code = parse_code(arguments);
            let result = compute_industry_position_impl(client, &code).await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        },
        "compute_bottleneck_signals" => {
            let code = parse_code(arguments);
            let result = compute_bottleneck_signals_impl(client, &code).await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        },
        // 输出契约与 mapper_prompt attention_metrics 字段对齐：
        //   coverage_change_3m / search_heat / relative_volume / consensus_gap / attention_score
        "compute_attention_score" => {
            let code = parse_code(arguments);
            let result = compute_attention_score_impl(client, &code).await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        },
        // 输出契约与 mapper_prompt exit_signals 字段对齐：
        //   technology_disruption_risk / capacity_oversupply_risk / new_entrant_risk
        //   demand_slowdown_risk / overall_exit_urgency
        "check_exit_signals" => {
            let code = parse_code(arguments);
            let entry_price = arguments["entry_price"].as_f64();
            let stop_loss_price = arguments["stop_loss_price"].as_f64();
            let result =
                check_exit_signals_impl(client, &code, entry_price, stop_loss_price).await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        },
        "verify_catalysts" => {
            let code = parse_code(arguments);
            let catalysts: Vec<String> = arguments["catalyst_descriptions"]
                .as_array()
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let result = verify_catalysts_impl(client, &code, &catalysts).await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        },
        "compute_serenity_performance" => {
            let code = parse_code(arguments);
            let recommend_date = parse_str(arguments, "recommend_date");
            let result = compute_serenity_performance_impl(client, &code, &recommend_date).await?;
            serde_json::to_string(&result).map_err(|e| e.to_string())
        },
        "optimize_attention_weights" => {
            let samples = arguments["samples"].as_array().cloned().unwrap_or_default();
            let result = optimize_attention_weights_impl(&samples);
            serde_json::to_string(&result).map_err(|e| e.to_string())
        },
        // P1-2 修复(2026-08-09): run_quality_gate 原只有 schema 声明、dispatch 无实现分支，
        // LLM 调用必走 Unknown MCP tool。现接入 astock-data::quality::run_quality_gate。
        // 输入: {reports_json: "{expert_id: report_text}"}，输出: {grade, summary, warnings}。
        "run_quality_gate" => {
            let reports_json = arguments["reports_json"].as_str().ok_or_else(|| {
                "run_quality_gate 缺少 reports_json 参数（{expert_id: report_text} JSON）"
                    .to_string()
            })?;
            let reports: std::collections::HashMap<String, String> =
                serde_json::from_str(reports_json)
                    .map_err(|e| format!("reports_json 解析失败: {e}"))?;
            let check = crate::quality::run_quality_gate(&reports);
            serde_json::to_string(&serde_json::json!({
                "grade": format!("{:?}", check.grade),
                "summary": check.summary,
                "warnings": check.warnings,
            }))
            .map_err(|e| e.to_string())
        },
        // G3 产业链相关工具（get_industry_chain_propagation /
        // map_news_to_cross_market_stocks）已于 P2-8 阶段迁至
        // `axagent_analysis_engine::mcp_tools::execute_industry_chain_tool`。
        // 调用方需在调用 astock-data::mcp_tools::execute_mcp_tool 之前，
        // 先尝试 axagent_analysis_engine::mcp_tools::execute_industry_chain_tool。
        _ => Err(format!("Unknown MCP tool: {tool_name}")),
    }
}

// ── 估值计算辅助函数 ──────────────────────────────────────────────────────

use axagent_harness::market_data::FinancialReport;

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// Piotroski F-Score (0-9)
///  profitability(4): 正ROE, 正经营现金流, ROE同比增长, 现金流>净利润
///  leverage(3): 长期负债不增, 流动比率提升, 无新股增发
///  efficiency(2): 毛利率提升, 资产周转率提升
fn compute_f_score(financials: &[FinancialReport]) -> u32 {
    if financials.is_empty() {
        return 0;
    }
    let curr = &financials[0];
    let prev = financials.get(1);
    let mut score = 0u32;

    // P1: 正 ROE（roe 是百分比值，>0 即正 ROE）
    if curr.roe.unwrap_or(0.0) > 0.0 {
        score += 1;
    }
    // P2: 正经营现金流
    if curr.operating_cash_flow.unwrap_or(0.0) > 0.0 {
        score += 1;
    }
    // P3: ROE 同比增长
    if let (Some(curr_roe), Some(prev_roe)) = (curr.roe, prev.and_then(|p| p.roe)) {
        if curr_roe > prev_roe {
            score += 1;
        }
    } else if curr.roe.unwrap_or(0.0) > 0.0 && prev.is_none() {
        score += 1; // 仅一期且为正 ROE 也算通过
    }
    // P4: 经营现金流 > 净利润（应计质量）
    let np = curr.net_profit.unwrap_or(0.0);
    let ocf = curr.operating_cash_flow;
    if let (Some(ocf_val), np_val) = (ocf, np) {
        if ocf_val > np_val {
            score += 1;
        }
    }

    // L1: 长期负债/资产负债率不增
    if let (Some(curr_dr), Some(prev_dr)) = (curr.debt_ratio, prev.and_then(|p| p.debt_ratio)) {
        if curr_dr <= prev_dr {
            score += 1;
        }
    }
    // L2: 流动比率提升
    if let (Some(curr_cr), Some(prev_cr)) = (curr.current_ratio, prev.and_then(|p| p.current_ratio))
    {
        if curr_cr >= prev_cr {
            score += 1;
        }
    } else if curr.current_ratio.unwrap_or(0.0) >= 1.0 {
        score += 1;
    }
    // L3: 无新股增发 — 用 net_profit/eps 比值近似股本变化；比值下降视为股本增加
    // 股本 = net_profit / eps，若股本增长则视为可能增发
    if let (Some(curr_np), Some(curr_eps), Some(prev_np), Some(prev_eps)) =
        (curr.net_profit, curr.eps, prev.and_then(|p| p.net_profit), prev.and_then(|p| p.eps))
    {
        if curr_eps > 0.0 && prev_eps > 0.0 {
            let curr_shares_approx = (curr_np / curr_eps).abs();
            let prev_shares_approx = (prev_np / prev_eps).abs();
            if curr_shares_approx <= prev_shares_approx * 1.05 {
                // 股本变化在 5% 以内视为无显著增发
                score += 1;
            }
        }
    } else if prev.is_none() {
        // 只有一期数据，检查当前资产负债率是否健康
        if curr.debt_ratio.unwrap_or(100.0) < 50.0 {
            score += 1;
        }
    }

    // E1: 毛利率提升
    if let (Some(curr_gm), Some(prev_gm)) = (curr.gross_margin, prev.and_then(|p| p.gross_margin)) {
        if curr_gm > prev_gm {
            score += 1;
        }
    } else if curr.gross_margin.unwrap_or(0.0) > 20.0 {
        score += 1;
    }
    // E2: 资产周转率提升 — 用 revenue / total_assets 近似；无 total_assets 时用营收同比增长代替
    if let (Some(curr_rev), Some(prev_rev)) = (curr.revenue, prev.and_then(|p| p.revenue)) {
        if let (Some(curr_ta), Some(prev_ta)) =
            (curr.total_assets, prev.and_then(|p| p.total_assets))
        {
            if curr_ta > 0.0 && prev_ta > 0.0 {
                let curr_tat = curr_rev / curr_ta;
                let prev_tat = prev_rev / prev_ta;
                if curr_tat > prev_tat {
                    score += 1;
                }
            } else if curr_rev > prev_rev {
                score += 1; // 营收增长近似替代周转率提升
            }
        } else if curr_rev > prev_rev {
            score += 1; // 营收增长近似替代周转率提升
        }
    }

    score.min(9)
}

/// 护城河量化评分 (0-100)
fn compute_moat_score(
    financials: &[FinancialReport],
    pe: Option<f64>,
    _pb: Option<f64>,
) -> (u32, &'static str) {
    if financials.is_empty() {
        return (0, "无");
    }
    let mut score = 0u32;

    // 1. ROE 持续性 (30分)
    let roe_values: Vec<f64> = financials.iter().take(5).filter_map(|r| r.roe).collect();
    let roe_count = roe_values.len() as f64;
    let avg_roe = if roe_count > 0.0 {
        roe_values.iter().sum::<f64>() / roe_count
    } else {
        0.0
    };
    if avg_roe > 20.0 {
        score += 30;
    } else if avg_roe > 15.0 {
        score += 20;
    } else if avg_roe > 10.0 {
        score += 10;
    }

    // 2. 毛利率稳定性 (20分)
    let gm_values: Vec<f64> = financials.iter().take(5).filter_map(|r| r.gross_margin).collect();
    let gm_count = gm_values.len() as f64;
    let avg_gm = if gm_count > 0.0 {
        gm_values.iter().sum::<f64>() / gm_count
    } else {
        0.0
    };
    if avg_gm > 60.0 {
        score += 20;
    } else if avg_gm > 40.0 {
        score += 15;
    } else if avg_gm > 20.0 {
        score += 8;
    }

    // 3. 低负债 (20分) — 使用多期平均负债率，避免单期异常
    let debt_values: Vec<f64> = financials.iter().take(5).filter_map(|r| r.debt_ratio).collect();
    let debt_count = debt_values.len() as f64;
    let avg_debt = if debt_count > 0.0 {
        debt_values.iter().sum::<f64>() / debt_count
    } else {
        100.0
    };
    if avg_debt < 20.0 {
        score += 20;
    } else if avg_debt < 40.0 {
        score += 15;
    } else if avg_debt < 60.0 {
        score += 8;
    }

    // 4. 盈利稳定性 (15分)
    let all_profitable = financials.iter().take(5).all(|r| r.net_profit.unwrap_or(-1.0) > 0.0);
    if all_profitable {
        score += 15;
    }

    // 5. 估值合理性 (15分)
    if let Some(pe_val) = pe {
        if pe_val < 15.0 && pe_val > 0.0 {
            score += 15;
        } else if pe_val < 25.0 {
            score += 10;
        } else if pe_val < 50.0 {
            score += 5;
        }
    }

    let level = if score >= 70 {
        "宽阔"
    } else if score >= 40 {
        "狭窄"
    } else {
        "无"
    };
    (score, level)
}

/// DCF 两阶段估值（保守/中性/乐观三档）
///
/// 估值参数说明（2026-09-12 与 `analysis-engine::decision::ValueConfig` 的 A 股校准值对齐）：
/// - 永续增长率 `PERPETUAL_GROWTH = 4%`：接近长期名义 GDP 增速（原 3% 偏低）
/// - 折现率 `DISCOUNT_RATE = 8.5%`：无风险利率 2.5% + 6% 风险溢价（原 10% 偏高）
/// - 默认增长率 `DEFAULT_GROWTH = 12%`：优秀公司平均增速（原 8% 偏保守，系统性低估成长股）
/// - 增长率区间 `[MIN_GROWTH, MAX_GROWTH] = [-30%, 30%]`：限制异常值
///
/// **下界由 `+2%` 改为 `-30%`（2026-09-12，PLAN P0-F 方案 A）**。原值 `+0.02` 是**正数**，
/// 而 `growth = revenue_yoy.clamp(min_growth, max_growth)` ⇒ **营收负增长的标的被抬成
/// 「确定性 +2% 增长」**，且**越差的公司偏置越大**。DB 实证（603353）：`revenue_yoy = −6.05%`
/// 被抬到 `+2%`，`dcf.mid` 因之偏高约 42%（修复后 5.15 → 3.61）。
/// 同源的 `compute_graham_value` 已于 P1-A 把下界由 `0.0` 改为 `-0.30`（其注释明写
/// 「把负增长抹平为 0……反而**高估**其内在价值」）—— 本次是补齐 DCF 侧的**漏修**。
const PERPETUAL_GROWTH: f64 = 0.04;
const DISCOUNT_RATE: f64 = 0.085;
const DEFAULT_GROWTH: f64 = 0.12;
const MIN_GROWTH: f64 = -0.30;
const MAX_GROWTH: f64 = 0.30;

/// 三档对**预测期增长率**的方向缩放因子（2026-09-12 新增）。
///
/// ⚠️ 不能无条件乘同一个数。原实现 `low_growth = growth × 0.6`、`high_growth = growth × 1.5`
/// 只在 `growth > 0` 时保证档位序；`growth < 0` 时 `× 0.6` 会**变大**（−6% → −3.6%），
/// 「保守档」反而比中性档乐观 ⇒ `low > mid`、`high < mid` 的**档位乱序**（实证：修复前
/// `MIN_GROWTH` 之所以被迫取正数，部分原因就是在掩盖这个乱序）。
///
/// 正确语义是「**向悲观/乐观方向缩放**」：
/// - 正增长：保守档缩到 60%，乐观档放到 150%
/// - 负增长：保守档**放大衰退**到 140%，乐观档**收敛衰退**到 60%
///
/// 由此得到不变量 `low_growth ≤ growth ≤ high_growth`（对任意符号的 `growth` 成立），
/// 已由 `dcf_tier_ordering_holds_for_negative_growth` 覆盖。
const LOW_GROWTH_SCALE_POS: f64 = 0.6;
const LOW_GROWTH_SCALE_NEG: f64 = 1.4;
const HIGH_GROWTH_SCALE_POS: f64 = 1.5;
const HIGH_GROWTH_SCALE_NEG: f64 = 0.6;

/// 保守档永续增长率下限：**不允许负永续增长**。
///
/// 原实现用 `min_growth / 2.0` 作下限，在 `MIN_GROWTH` 转负后会变成 `-0.15`
/// —— 等于允许「公司永久萎缩」的终值假设，DCF 终值项失去经济含义。
/// 故提为独立常量并固定在 0（`perpetual_growth` 本身经 `pct()` 守卫恒为正，此下限只是兜底）。
const MIN_PERPETUAL_GROWTH: f64 = 0.0;
/// 乐观档永续增长率上限：`high_perpetual = min(p × 1.3, MAX_PERPETUAL_GROWTH)`。
///
/// 2026-09-12 由内联魔数 `0.05` 提为具名常量 —— 路径 Y 把 `PERPETUAL_GROWTH`
/// 对齐到 0.04 后，`0.04 × 1.3 = 0.052` **触顶**该上限，乐观档输出对上限取值
/// 异常敏感（002353 实测 high 97.89 → 167.45，+71%，远超中性档 +52%），故显式化。
///
/// 不变量：必须 < `DISCOUNT_RATE`，否则 `terminal_spread = (d − p).max(0.001)`
/// 的兜底分支会让终值失真（0.05 < 0.085 ✓）。
const MAX_PERPETUAL_GROWTH: f64 = 0.05;
const FORECAST_YEARS: i32 = 5;

// ── DCF 模型适用性判据（2026-09-14）─────────────────────────────────────────
//
// ## 为什么不按行业写
//
// 601166（兴业银行）的 DCF 给出 40.17–49.26 元（现价 18.15，`upsidePct = 143.1`），
// 与同一份输出里的「观望 + 0% 仓位」直接打脸。根因是 DCF 的**前提假设对银行不成立**：
// 银行没有「企业自由现金流」概念（存款/贷款是经营原料，不是可分配现金），
// 走 fallback 拿「近 5 年净利均值 × 0.90 = 730.5 亿」当 FCF 折现，必然给出荒谬高值。
//
// 但**修法不能是「if 银行 then 跳过」** —— 那样只是把地鼠从这个洞赶到下一个洞：
// 保险（负债 90%+）、券商（保证金负债）、地产（预收款 = 负债）、
// 重资产周期（折旧巨大 ⇒ FCF 与净利背离）全都命中同一类错误，
// 而它们的行业名各不相同、vendor 返回的粒度还不一致（实测是「金融」而非「银行」）。
//
// ⇒ 锚点必须是**数据形态**：DCF 不成立的信号与行业标签无关，可观测：
//   ① 杠杆畸高 —— 净利由权益乘数驱动，企业 FCF 口径不成立
//   ② FCF 与净利背离 —— 现金流不反映股东可分配（金融/地产/重资产周期典型）
//   ③ 预测期收缩却靠永续正增长撑估值 —— 结论由永续假设独裁
//      （该矛盾已由「永续增长率符号一致性约束」直接修复；本条判据用于标记
//       「即便永续归零，估值仍几乎全由终值贡献」的残余形态）
//
// 输出形态：`applicable` + `inapplicable_reason` + `applicability_signals[]`，
// **数值仍照常计算**（零破坏性，旧模板依赖三档值），由下游主动降级。
//
// 这样 601166 自动命中（负债 91.6% + 净利为正而 FCF ≤ 0），
// 茅台式标的（负债 ~20%、FCF/净利 > 1）不命中，
// 地产公司（负债 80%）自动命中 —— **无需维护任何行业白名单**。

/// 判据 ①：资产负债率上限（%）。超过即认为「净利由杠杆驱动」。
///
/// 取值依据：非金融 A 股负债率中位约 40–50%，80% 已是显著离群；而金融/地产的
/// 负债率天然在 80–93%（银行 91.6% 实测）⇒ 该阈值同时具备「识别金融/地产」
/// 与「不误伤制造业」的能力，且不依赖行业字段是否可用。
const LEVERAGE_INAPPLICABLE_PCT: f64 = 80.0;

/// 判据 ②：净利与自由现金流的背离。**两种形态**任一即命中：
///
/// - **符号相反**（净利 > 0 而 FCF ≤ 0）——最强信号。账面盈利但经营/投资现金流合
///   计净流出，说明「利润不是能拿走的现金」，FCF 折现的分子本身没有经济含义。
///   银行/保险/地产/扩张期重资产公司典型。
/// - **量级脱钩**（两者皆正但 `FCF/净利 < 0.3`）——现金流跟不上盈利。
///
/// 阈值依据：正常经营企业该比值在 0.7–1.2（折旧与资本开支大致抵消）；
/// 0.3 是「明显脱钩」与「波动但相关」的分界。
///
/// ⚠️ 当期 FCF **数据缺失**（`None`）不算命中 —— 缺数据 ≠ 模型不成立，
/// 该情形由 fallback 锚定 + `is_fallback_anchor` 承担置信度衰减。
const FCF_NP_DIVERGENCE_MIN: f64 = 0.3;

/// 判据 ③：终值现值占比上限。超过且预测期负增长 ⇒ 结论由永续假设独裁。
///
/// 601166 实测：**符号一致性约束之前**终值占 70.6%（`growth = −0.25%` + 永续 +3%），
/// 该矛盾已由约束修复；约束后永续压到 0，占比降到 **62.0%** ⇒
/// **③ 对 601166 不再触发**（它由 ①② 判定，见 `applicable` 字段注释）。
/// 本条判据保留用于标记「即便永续已归零、估值仍几乎全由永续假设贡献」的形态
/// —— 这类标的的预测期增长率本身就是噪声，结论不可检验。
///
/// 占比取**生效值**（约束后），因为那才是真正流向下游与面板的结论的构成。
const TERMINAL_RATIO_MAX: f64 = 0.7;

/// 估值运行时配置（可由前端设置页下发）
///
/// 当 `Some(config)` 传入时使用自定义值，否则回退到模块级常量。
#[derive(Debug, Clone, Copy, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValuationConfig {
    pub perpetual_growth: Option<f64>,
    pub discount_rate: Option<f64>,
    pub default_growth: Option<f64>,
    pub min_growth: Option<f64>,
    pub max_growth: Option<f64>,
    pub forecast_years: Option<i32>,
    pub bond_yield: Option<f64>,
}

impl ValuationConfig {
    fn perpetual_growth(&self) -> f64 {
        self.perpetual_growth.unwrap_or(PERPETUAL_GROWTH)
    }
    fn discount_rate(&self) -> f64 {
        self.discount_rate.unwrap_or(DISCOUNT_RATE)
    }
    fn default_growth(&self) -> f64 {
        self.default_growth.unwrap_or(DEFAULT_GROWTH)
    }
    fn min_growth(&self) -> f64 {
        self.min_growth.unwrap_or(MIN_GROWTH)
    }
    fn max_growth(&self) -> f64 {
        self.max_growth.unwrap_or(MAX_GROWTH)
    }
    fn forecast_years(&self) -> i32 {
        self.forecast_years.unwrap_or(FORECAST_YEARS)
    }
    fn bond_yield(&self) -> f64 {
        self.bond_yield.unwrap_or(4.4)
    }

    /// 从**扁平参数**构造估值配置（C2 路径 Z，2026-09-12）。
    ///
    /// 背景：`ToolNodeConfig.input_mapping` 是 `HashMap<String, String>`，dispatcher 把
    /// value 当**变量名**查（`crates/rt-workflow/src/work_engine/dispatcher.rs:463-466` 的 `context.variables.get(v)`），
    /// **无法构造嵌套 object**。因此模板侧拼不出 `valuation_config` object，
    /// 设置面板的 `value_dcf_*` 变量只能改用扁平参数承接。
    ///
    /// **单位约定（PLAN 决策 D1）**：扁平参数一律为**百分数**口径，与面板变量默认值
    /// （`value_dcf_growth_rate=12.0` / `perpetual=4.0` / `discount=8.5`）一致，
    /// 在此**单点**换算为小数。禁止在调用方（模板 / 前端）做 `/100` —— 否则
    /// 本函数会成为第二处口径，两处一旦漂移就是「折现率 850%」级事故。
    ///
    /// **取值守卫**：越界项（非有限 / ≤0 / >100）一律**忽略该项**（回退模块常量默认），
    /// 避免污染。任一有效项存在即返回 `Some`，否则 `None`（回退常量默认）。
    ///
    /// 优先级：`valuation_config`(object) > 本函数(扁平) > 模块常量（见 `compute_valuation`）。
    fn from_flat_arguments(arguments: &serde_json::Value) -> Option<Self> {
        /// 读取百分数参数并换算为小数；缺失/越界返回 None。
        fn pct(arguments: &serde_json::Value, key: &str) -> Option<f64> {
            let raw = arguments.get(key)?.as_f64()?;
            (raw.is_finite() && raw > 0.0 && raw <= 100.0).then(|| raw / 100.0)
        }
        let default_growth = pct(arguments, "dcf_growth_rate");
        let perpetual_growth = pct(arguments, "dcf_perpetual_rate");
        let discount_rate = pct(arguments, "dcf_discount_rate");
        if default_growth.is_none() && perpetual_growth.is_none() && discount_rate.is_none() {
            return None;
        }
        Some(Self {
            perpetual_growth,
            discount_rate,
            default_growth,
            min_growth: None,
            max_growth: None,
            forecast_years: None,
            bond_yield: None,
        })
    }
}

/// 近 N 个年报（report_date 含 "-12-31"，兼容 "2025-12-31" 与 "2025-12-31 00:00:00" 两种格式）
/// 中的正净利润均值。用于亏损期（周期底部）的归一化估值锚定。
///
/// 背景：vendor 返回的 reports 按报告期倒序（[0] 为最新），季报净利润为累计值，
/// 直接对全部报告期取均值会混淆季度口径；过滤年报天然规避该问题。
fn normalized_annual_profit(financials: &[FinancialReport], max_years: usize) -> Option<f64> {
    let vals: Vec<f64> = financials
        .iter()
        .filter(|r| r.report_date.contains("-12-31"))
        .take(max_years)
        .filter_map(|r| r.net_profit)
        .filter(|np| *np > 0.0)
        .collect();
    if vals.is_empty() {
        None
    } else {
        Some(vals.iter().sum::<f64>() / vals.len() as f64)
    }
}

/// 近 N 个年报中的正 EPS 均值（元/股），用于亏损期格雷厄姆公式的归一化锚定
fn normalized_annual_eps(financials: &[FinancialReport], max_years: usize) -> Option<f64> {
    let vals: Vec<f64> = financials
        .iter()
        .filter(|r| r.report_date.contains("-12-31"))
        .take(max_years)
        .filter_map(|r| r.eps)
        .filter(|e| *e > 0.0)
        .collect();
    if vals.is_empty() {
        None
    } else {
        Some(vals.iter().sum::<f64>() / vals.len() as f64)
    }
}

/// 报告期键：取 `report_date` 前 10 字符（`YYYY-MM-DD`）。
/// 兼容 `"2025-12-31"` 与 `"2025-12-31 00:00:00"` 两种格式；
/// 非 10 位合法日期（长度不足 / 分隔符错位 / 非 UTF-8 边界）返回 None。
fn report_period_key(report_date: &str) -> Option<&str> {
    let key = report_date.get(..10)?;
    if key.as_bytes()[4] == b'-' && key.as_bytes()[7] == b'-' {
        Some(key)
    } else {
        None
    }
}

/// 年报口径 EPS（元/股）——格雷厄姆公式的输入。
///
/// P1-A 修复（2026-09-11）：vendor 的 `financials[0]` 是**最新报告期**，
/// 中报/季报的 `eps` 是**年内累计值**而非 TTM。原实现直接把它当年度值代入，
/// 系统性低估内在价值约 2 倍。DB 实证（002353 杰瑞股份 2026-06-30）：
/// `eps = 1.18`（H1 半年累计），`EPS × 8.5 × 4.4/4.4 = 10.03`；而东财原始
/// 数据显示 TTM EPS = 2.60（2025FY 2.64 + 2026H1 1.18 − 2025H1 1.22），
/// 应为 22.10。该错误值随后被 LLM 当作核心看空论据（`buffett_verdict`
/// 【减持】与 `bear_points` 均引用「格雷厄姆下行 91.6%」）。
///
/// 取值链：
///   ① 最新为年报（`-12-31`）→ 直接使用正 EPS
///   ② 最新为季报/中报（EPS 为累计值）→ TTM = 上年年报 EPS + 本期累计 EPS
///                                            − 上年同期累计 EPS
///   ③ ① ② 不可用，或 TTM ≤ 0（亏损 / 负增长极端）→ 近 5 年报正 EPS 均值
///   ④ 全部不可用 → None（公式不适用，不以 0 冒充估值）
fn annualized_eps(financials: &[FinancialReport]) -> Option<f64> {
    let latest = financials.first()?;

    // ① 年报口径本身就是年度值（EPS ≤ 0 时仍走归一化）
    if latest.report_date.contains("-12-31") {
        return latest.eps.filter(|e| *e > 0.0).or_else(|| normalized_annual_eps(financials, 5));
    }

    // ② 季报/中报：由「上年年报 + 本期累计 − 上年同期累计」还原 TTM
    let ttm = latest.eps.and_then(|latest_eps| {
        let key = report_period_key(&latest.report_date)?;
        let year = key.get(..4)?.parse::<i32>().ok()?;
        let month_day = key.get(4..)?; // 形如 "-06-30"
        let prev_year = year.checked_sub(1)?;
        let find_eps = |target: &str| {
            financials
                .iter()
                .find(|r| report_period_key(&r.report_date) == Some(target))
                .and_then(|r| r.eps)
        };
        let annual = find_eps(&format!("{prev_year}-12-31"))?;
        let prev_same_period = find_eps(&format!("{prev_year}{month_day}"))?;
        Some(annual + latest_eps - prev_same_period)
    });

    // ③ 兜底：TTM 不可得或非正 → 近 5 年报正 EPS 均值（周期底部锚定）
    match ttm {
        Some(v) if v > 0.0 => Some(v),
        _ => normalized_annual_eps(financials, 5),
    }
}

/// 年度口径 ROE（%）—— 与 [`annualized_eps`] 同构，保证同一份报告里盈利用同一口径。
///
/// ## 为什么必须还原（2026-09-14 601166 实证）
///
/// vendor 的 `financials.first().roe` 在**中报/季报口径下是「年内累计值」**，
/// 而同一份报告里的 `pe` 用的是**年化 EPS**。两者并列即产生自相矛盾的结论：
///
/// ```text
/// 601166 兴业银行 2026-06-30
///   roe = 4.8   ← 半年累计（411.31 亿 / 净资产 ~8350 亿 = 4.93%）
///   pe  = 5.09  ← 年化（3841 亿市值 / ~755 亿年化净利）
/// ```
///
/// 于是「估值极低」与「盈利严重恶化」在同一份输出里并存，且**整条看空链都建立
/// 在 4.8% 上**：`a-fundamentals`（bear_points 首条「ROE 严重下滑至 4.8%」）、
/// `risk-agg`/`risk-neu`/`risk-con`、`research-mgr`（「严重偏离行业均值 10-12%」）、
/// `debate-convergence`（`decisive_bear_acks` 首条）全部复述该数字。
/// 而年化后 ≈ 9.8%，**正落在银行业正常区间下沿** —— 结论方向被这一个口径错误翻转。
///
/// 该值还直接进风险判定：`portfolio-mgr.rhai` 的 `is_high` 含
/// `roe < RISK_ROE_HIGH(5.0)`，4.8 恰好触发而 9.8 不会。
///
/// ## 取值链
///
/// 1. 最新为年报（`-12-31`）→ 直接使用
/// 2. 中报/季报 → `上年年报 ROE + 本期累计 ROE − 上年同期累计 ROE`（TTM 还原）
/// 3. 缺上年同期 → 按报告期月份数粗年化（`ROE × 12 / months`），
///    对银行/保险等利润均匀行业准确，对强季节性行业是近似
/// 4. 全部不可用 → `None`（**保留原值**，不以 0 或猜测值冒充）
pub(crate) fn annualized_roe(financials: &[FinancialReport]) -> Option<f64> {
    let latest = financials.first()?;

    // ① 年报口径本身就是年度值
    if latest.report_date.contains("-12-31") {
        return latest.roe;
    }

    let latest_roe = latest.roe?;
    let key = report_period_key(&latest.report_date)?;
    let year = key.get(..4)?.parse::<i32>().ok()?;
    let month_day = key.get(4..)?; // 形如 "-06-30"
    let prev_year = year.checked_sub(1)?;

    let find_roe = |target: &str| {
        financials
            .iter()
            .find(|r| report_period_key(&r.report_date) == Some(target))
            .and_then(|r| r.roe)
    };

    // ② TTM 还原：与 annualized_eps 完全同构的「加年报、减同期」法。
    //    ROE 的分母（净资产）在半年内变动 <6%，相减误差小于不做年化的 2 倍偏差。
    if let Some(annual) = find_roe(&format!("{prev_year}-12-31")) {
        if let Some(prev_same) = find_roe(&format!("{prev_year}{month_day}")) {
            return Some(annual + latest_roe - prev_same);
        }

        // ③ 有上年年报但缺上年同期 → 按月份数粗年化
        let months = key.get(5..7)?.parse::<f64>().ok()?;
        if months > 0.0 && months < 12.0 {
            return Some(latest_roe * 12.0 / months);
        }
    }

    // ④ 无法还原 —— 返回 None，由调用方决定保留原值还是标记不可用
    None
}

/// DCF 两阶段估值（保守/中性/乐观三档）
///
/// V74(2026-09-10) 返回值语义变更：
/// - `Some((三档值, 口径说明))`：估值有效
/// - `None` + 原因：估值不可用（无股本 / 当期FCF≤0 且近5年报无正净利年度）
///
/// 旧实现把「不可用」编码成 `(0,0,0)`，下游把 0 当成真实估值，
/// 产出「理想买入价 0 元 / 安全边际 0% / DCF 三档全 0」级退化输出。
/// DCF **实际生效参数**快照（2026-09-12 新增，随 `dcf.assumptions` / `dcf_valuation.assumptions` 落库）。
///
/// 背景（AUDIT §7.4.6）：`603353` 的 `dcf.mid` 在 P0-E 修复前后为 `2.18 → 5.15`，
/// 而**参数注入侧完全没变** —— 变的只有 FCF 锚定值（2926.6 万 → 6918.0 万）。
/// 但旧输出只落三档数值、不落参数，对账只能靠「反解」：
/// 三档给出 3 个方程，而待定参数是 `(growth, perpetual, discount, anchor, shares)`
/// **5 个未知量** ⇒ 欠定。实测三组差异很大的锚定假设能同时「命中」同一份存档值
/// （误差 ~0.5%），**反解结果不可作证据**。
///
/// 本结构把参数**直接输出**，使「估值可否复现」从反解（欠定）降级为对账（唯一解）。
/// 落库后任何一次 `2.18 vs 5.15` 级别的疑案都可一眼判定位：数据侧还是参数侧。
///
/// 关键不变量（`growth` 是取值链最后一环，也是最易误判的一环）：
/// `growth = revenue_yoy / 100`（缺失时回落 `default_growth`）**再统一 clamp 到
/// `[min_growth, max_growth]`** —— 两条分支都过 clamp（2026-09-12 方案 A 修正，
/// 原实现只在 `revenue_yoy` 存在时 clamp）。即模板变量 `dcf_growth_rate` **不直接决定
/// `growth`**，它只填 `default_growth` 这个兜底槽（实测：`dcf_growth_rate=0` 被
/// `from_flat_arguments` 的 `raw > 0.0` 守卫丢弃，`growth` 仍由 `revenue_yoy` 决定）。
///
/// 三档增长率由 `growth` **按方向缩放**派生（见 `LOW_GROWTH_SCALE_*`）：
/// 正增长时 `low = 0.6g` / `high = 1.5g`；负增长时 `low = 1.4g`（更悲观）/
/// `high = 0.6g`（更乐观），恒有 `low_growth ≤ growth ≤ high_growth`。
#[derive(Debug, Clone, serde::Serialize)]
struct DcfAssumptions {
    /// 中性档预测期增长率（小数）
    growth: f64,
    /// 保守档预测期增长率（向悲观方向缩放：`growth ≥ 0` 时 ×0.6，`growth < 0` 时 ×1.4）
    low_growth: f64,
    /// 乐观档预测期增长率（向乐观方向缩放：`growth ≥ 0` 时 ×1.5，`growth < 0` 时 ×0.6，再 clamp）
    high_growth: f64,
    /// 中性档永续增长率（小数）
    perpetual_growth: f64,
    /// 保守档永续增长率（`perpetual × 0.7`）
    low_perpetual: f64,
    /// 乐观档永续增长率（`perpetual × 1.3`，上界 `MAX_PERPETUAL_GROWTH`）
    high_perpetual: f64,
    /// 折现率（小数）
    discount_rate: f64,
    /// 预测期年数
    forecast_years: i32,
    /// FCF 锚定值（元）—— ① 当期 FCF，或 ② 近 5 年报正净利均值 × 0.90
    fcf_anchor: f64,
    /// 每股 FCF（元/股）
    fcf_per_share: f64,
    /// 总股本（股）
    total_shares: f64,
    /// 口径说明（`fcf_anchor` 的来源），直接复用 `dcf.note`
    basis: String,
    /// 锚定是否来自**归一化 fallback**（`当期 FCF ≤ 0` ⇒ 近 5 年报正净利均值 × 0.90）
    ///
    /// 2026-09-12（P0-I）新增，消费者是 `portfolio-mgr.rhai` 的 f5 估值因子。
    /// 该因子用它做**置信度衰减**：历史均值代理出来的锚定是回溯且偏低的，
    /// 给成长/转型标的定价会系统性低估，不应与「当期真实 FCF」拿同等的信号强度。
    ///
    /// 603353 实证（模板 v37，样本 997cdf50）：`basis` 命中该 fallback，
    /// `dcf.upsidePct = −90.8` ⇒ `f5 σ = −0.696`，权重 0.21 为全因子最大
    /// （再乘 `regime_mod 1.4`）⇒ **该因子单独贡献 −0.1462，占净负贡献
    /// Σ(σ·w)=−0.1688 的 87%** —— 即负向证据几乎全来自一个模型自认退化的锚定。
    ///
    /// ⚠️ 但实测反证过「修它就能翻转 action」：k=0.5 → eff 0.3027→0.3318（仍减持）；
    /// k=0（σ 完全中性化）→ 0.3610（仍减持）；把全部 `σ < 0` 因子归零上限也只有
    /// eff=0.4327（观望），距 `ACTION_HOLD_THRESHOLD`(0.48) 差 0.047。
    /// 故该字段的用途是**置信度纠正**，不是决策翻转的开关。
    is_fallback_anchor: bool,
    /// **模型适用性**：DCF 的前提假设是否对本标成立（2026-09-14 新增）。
    ///
    /// `false` 表示「本标的不满足 DCF 的前提」—— 数值仍会算出（保持零破坏性，
    /// 下游有旧模板依赖三档值），但**下游不得把它当作可靠估值证据**。
    ///
    /// 判据锚定**数据形态**而非行业标签，详见模块顶部常量区的说明。命中任一即 `false`：
    /// ① `debt_ratio > LEVERAGE_INAPPLICABLE_PCT(80)` —— 净利由杠杆驱动
    /// ② 净利为正但当期真实 FCF ≤ 0（符号相反），或 `0 < FCF/净利 < 0.3`（量级脱钩）
    /// ③ `growth < 0 && 终值现值占比 > TERMINAL_RATIO_MAX(0.7)` —— 结论由永续假设独裁
    ///
    /// ## 601166 实证（2026-09-14，样本 86c7d441）
    ///
    /// 现价 18.15 元，DCF 给 `low/mid/high = 40.17/44.13/49.26`（`upsidePct = 143.1`），
    /// 而 LLM 层 `llmAction = 观望` / `positionPct = 0` —— 同一份输出里两个结论互相打脸。
    /// 复算 `mid = 44.13` 精确复现，其中 70.6% 来自永续终值，
    /// 且 `growth = −0.25%` 与 `perpetual_growth = +3%` 并存（模型自相矛盾）。
    ///
    /// 本字段为 `false`，命中 **①（91.6% > 80%）与 ②（净利为正、当期 FCF ≤ 0）
    /// 两条**；③ 未命中（符号一致性约束后终值占比降到 62.0%）。
    applicable: bool,
    /// 不适用的原因（多条以 `；` 连接）；`applicable == true` 时为 `None`。
    inapplicable_reason: Option<String>,
    /// 命中的判据清单（结构化，供诊断与面板逐条展示）
    applicability_signals: Vec<String>,
    /// 永续增长率是否因「预测期负增长」被符号一致性约束压回（2026-09-14 修复）。
    ///
    /// `true` 表示配置里给的 `perpetual_growth > 0` 但预测期 `growth < 0`，
    /// 实际生效值已被压到 0 —— 否则等于假设「5 年持续萎缩后第 6 年起永久正增长」。
    perpetual_clamped_by_negative_growth: bool,
    /// 配置里原本设定的永续增长率（未经一致性约束），仅当被压回时与 `perpetual_growth` 不同
    configured_perpetual_growth: f64,
    /// 中性档「终值现值 / 总现值」占比（0–1）。越高说明估值越依赖永续假设。
    terminal_value_ratio: f64,
}

fn compute_dcf(
    financials: &[FinancialReport],
    total_shares: Option<f64>,
    _current_price: f64,
    config: Option<&ValuationConfig>,
) -> (Option<(f64, f64, f64)>, String, Option<DcfAssumptions>) {
    if financials.is_empty() {
        return (None, "无财务数据，DCF不可用".to_string(), None);
    }
    let cfg = config.copied().unwrap_or_default();
    let perpetual_growth = cfg.perpetual_growth();
    let discount_rate = cfg.discount_rate();
    let default_growth = cfg.default_growth();
    let min_growth = cfg.min_growth();
    let max_growth = cfg.max_growth();
    let forecast_years = cfg.forecast_years();

    let latest = &financials[0];
    let shares = match total_shares {
        Some(s) if s > 0.0 => s,
        _ => return (None, "总股本不可用，DCF不可用".to_string(), None),
    };

    // vendor 返回的财务数据单位均为"元"，无需缩放
    // V74 FCF 取值链:
    //   ① 当期 FCF > 0 → 直接使用（free_cash_flow / ocf-capex）
    //   ② 当期 FCF ≤ 0（亏损期/周期底部）→ 近 5 年报正净利均值 × 0.90 归一化锚定
    //      （沿用原 net_profit×0.90 估算惯例，季报为累计值故只取年报口径）
    //   ③ 近 5 年报无正净利年度（持续亏损）→ 返回 None，DCF 不适用
    // P0-I(2026-09-12): fallback 锚定的口径文案提为常量 —— 既用于 `fcf_basis`，
    //   也用于给下游输出 `is_fallback_anchor` 布尔量（避免靠字符串前缀匹配判分支）。
    const FCF_FALLBACK_BASIS: &str = "当期FCF≤0，改用近5年报正净利均值×0.90归一化锚定（周期底部）";
    // 2026-09-14：`direct_fcf` 提到外层作用域 —— 适用性判据 ②（`FCF/净利` 背离）
    // 需要看到**当期真实** FCF，而不是 fallback 后的代理值（代理值恒 ≈0.9×净利，
    // 会把「符号相反」这个最强信号抹掉）。
    let direct_fcf = latest.free_cash_flow.or_else(|| {
        latest
            .operating_cash_flow
            .and_then(|ocf| latest.capital_expenditure.map(|capex| ocf - capex))
    });
    let (fcf, fcf_basis) = match direct_fcf {
        Some(v) if v > 0.0 => (v, "当期FCF".to_string()),
        _ => match normalized_annual_profit(financials, 5) {
            Some(avg_np) => (avg_np * 0.90, FCF_FALLBACK_BASIS.to_string()),
            None => {
                return (
                    None,
                    "当期FCF≤0且近5年报无正净利年度（持续亏损），DCF模型不适用".to_string(),
                    None,
                )
            },
        },
    };
    // P0-I(2026-09-12): 锚定来源标记，随 `DcfAssumptions` 落库，供 f5 做置信度衰减。
    let is_fallback_anchor = fcf_basis == FCF_FALLBACK_BASIS;
    let fcf_per_share = fcf / shares; // 元/股

    // 用营收同比增速作为 growth_rate 参考；缺省回落 `default_growth`。
    // 2026-09-12（P0-F 方案 A）：**两条分支统一 clamp**。原实现只在 `revenue_yoy`
    // 存在时 clamp，`unwrap_or(default_growth)` 分支不 clamp ⇒ 用户把扁平参数
    // `dcf_growth_rate` 配成 >30% 时 `growth` 超出 `MAX_GROWTH`，而 `high_growth`
    // 在下一步被 clamp 回 `MAX_GROWTH` ⇒ `high < mid` 的**档位乱序**（静默无日志）。
    // 统一 clamp 后 `growth ∈ [min_growth, max_growth]` 成为不变量，三档才单调。
    let growth = latest
        .revenue_yoy
        .map(|y| y / 100.0)
        .unwrap_or(default_growth)
        .clamp(min_growth, max_growth);

    // ── 永续增长率符号一致性约束（2026-09-14，用户裁决「直接强制」）────────────
    //
    // `growth` 来自 `revenue_yoy`（可为负），而 `perpetual_growth` 来自配置
    // （经 `pct()` 守卫恒为正）⇒ 两者可以符号相反。601166 实测 `growth = −0.25%`
    // 与 `configured_perpetual_growth = +3%` 并存，语义是「未来 5 年持续萎缩，
    // 但第 6 年起永久正增长」—— 这不是保守或乐观，是**模型自相矛盾**，
    // 且因为终值占 mid 的 70.6%，整个估值结论由这条矛盾假设独裁。
    //
    // 判据与行业无关（任何负增长标的都不该配正永续增长率），故直接强制：
    // 预测期负增长 ⇒ 永续增长率压到 ≤ 0。原配置值记入
    // `configured_perpetual_growth` 供面板/诊断对比，不静默丢弃。
    let configured_perpetual_growth = perpetual_growth;
    let perpetual_growth = if growth < 0.0 {
        perpetual_growth.min(0.0)
    } else {
        perpetual_growth
    };
    let perpetual_clamped_by_negative_growth =
        (perpetual_growth - configured_perpetual_growth).abs() > f64::EPSILON;

    // 两阶段 DCF，返回 `(总现值, 永续终值现值)`。
    //
    // 2026-09-14：由返回 `f64` 改为返回二元组 —— 适用性判据 ③ 需要「终值现值 /
    // 总现值」占比（越接近 1 说明结论越依赖永续假设、越不依赖可验证的预测期）。
    // ⚠️ 这里必须用 `//` 而非 `///`：`dcf_two_stage` 是 `let` 绑定的闭包语句，
    //   rustdoc 不为语句生成文档 ⇒ `///` 触发 `unused_doc_comments` warning，
    //   而 CI 跑 `clippy -D warnings` ⇒ 直接失败。
    let dcf_two_stage = |fcf_ps: f64, g: f64, p: f64, d: f64| -> (f64, f64) {
        let mut pv = 0.0;
        let mut current_fcf = fcf_ps;
        for year in 1..=forecast_years {
            current_fcf *= 1.0 + g;
            pv += current_fcf / (1.0 + d).powi(year);
        }
        let terminal_fcf = current_fcf * (1.0 + p);
        let terminal_spread = (d - p).max(0.001);
        let terminal_value = terminal_fcf / terminal_spread;
        let terminal_pv = terminal_value / (1.0 + d).powi(forecast_years);
        (pv + terminal_pv, terminal_pv)
    };

    // 保守档：增长率**向悲观方向**缩放，永续增长率打 7 折
    let low_growth = if growth >= 0.0 {
        growth * LOW_GROWTH_SCALE_POS
    } else {
        growth * LOW_GROWTH_SCALE_NEG
    };
    let low_perpetual = (perpetual_growth * 0.7_f64).max(MIN_PERPETUAL_GROWTH);
    let (low, _) = dcf_two_stage(fcf_per_share, low_growth, low_perpetual, discount_rate);

    // 中性档：原始增长率（已 clamp 到 `[min_growth, max_growth]`）与永续增长率
    let mid_growth = growth;
    let (mid, mid_terminal_pv) =
        dcf_two_stage(fcf_per_share, mid_growth, perpetual_growth, discount_rate);

    // 乐观档：增长率**向乐观方向**缩放，永续增长率放大 1.3 倍
    let high_growth = (if growth >= 0.0 {
        growth * HIGH_GROWTH_SCALE_POS
    } else {
        growth * HIGH_GROWTH_SCALE_NEG
    })
    .clamp(min_growth, max_growth);
    let high_perpetual = (perpetual_growth * 1.3_f64).min(MAX_PERPETUAL_GROWTH);
    let (high, _) = dcf_two_stage(fcf_per_share, high_growth, high_perpetual, discount_rate);

    // 终值现值占中性档估值的比例（0–1）。取**生效值**（符号一致性约束之后），
    // 因为它才是真正流向下游与面板的那个结论的构成。
    let terminal_value_ratio = if mid.abs() > f64::EPSILON {
        mid_terminal_pv / mid
    } else {
        0.0
    };

    // ── DCF 模型适用性（2026-09-14）────────────────────────────────────────
    //
    // 输出 `applicable` / `inapplicable_reason` / `applicability_signals` 三个字段，
    // 供下游（`portfolio-mgr.rhai` 的 f5 估值因子 + 诊断面板）**主动降级**。
    // 数值本身仍照常计算（零破坏性：旧模板依赖三档值），
    // 但 `applicable == false` 时下游不得把它当可靠估值证据。
    //
    // ⚠️ 判据锚定**数据形态**，不锚定行业标签 —— 见模块常量区
    // `LEVERAGE_INAPPLICABLE_PCT` 上方的长注释（为什么不能写 if 银行 then 跳过）。
    let mut applicability_signals: Vec<String> = Vec::new();

    // ① 杠杆畸高：净利由权益乘数驱动，企业自由现金流口径不成立
    if let Some(dr) = latest.debt_ratio.filter(|v| *v > LEVERAGE_INAPPLICABLE_PCT) {
        applicability_signals.push(format!(
            "资产负债率 {dr:.1}% > {LEVERAGE_INAPPLICABLE_PCT:.0}%：净利由杠杆驱动，\
             企业自由现金流口径不成立"
        ));
    }

    // ② FCF 与净利背离：现金流不反映股东可分配
    if let Some(np) = latest.net_profit.filter(|v| *v > 0.0) {
        match direct_fcf {
            // 符号相反是**最强信号** —— 账面盈利但现金净流出，FCF 折现无意义
            Some(v) if v <= 0.0 => applicability_signals.push(format!(
                "当期净利 {:.2} 亿为正但自由现金流 {:.2} 亿 ≤ 0（符号相反）：\
                 FCF 折现不反映股东可分配",
                np / 1e8,
                v / 1e8
            )),
            Some(v) => {
                let ratio = v / np;
                if ratio < FCF_NP_DIVERGENCE_MIN {
                    applicability_signals.push(format!(
                        "FCF/净利 = {ratio:.2} < {FCF_NP_DIVERGENCE_MIN}：\
                         现金流与盈利质量明显脱钩"
                    ));
                }
            },
            // 当期 FCF 数据缺失：**不**判不适用（缺数据 ≠ 模型不成立），
            // 由 fallback 锚定 + `is_fallback_anchor` 承担置信度衰减。
            None => {},
        }
    }

    // ③ 负增长却靠永续假设撑估值：结论由永续假设独裁
    if growth < 0.0 && terminal_value_ratio > TERMINAL_RATIO_MAX {
        applicability_signals.push(format!(
            "终值现值占估值 {:.1}% > {:.0}% 且预测期增长为负：\
             结论由永续假设独裁",
            terminal_value_ratio * 100.0,
            TERMINAL_RATIO_MAX * 100.0
        ));
    }

    let applicable = applicability_signals.is_empty();
    let inapplicable_reason = (!applicable).then(|| applicability_signals.join("；"));

    // 2026-09-12（P0-F 方案 A）**已修**：`growth` 下界由 `+0.02` 改为 `-0.30`，
    // 三档缩放改为「按方向」进行。档位序不变量 `low ≤ mid ≤ high` 现对任意符号的
    // `growth` 成立（此前 `MIN_GROWTH` 取正数部分原因是在掩盖 `growth < 0` 时的乱序）。
    // 603353 实证：`low/mid/high` 由 4.54/5.15/5.99 → **2.97/3.61/4.46**（`mid` −29.9%）。
    (
        Some((low, mid, high)),
        fcf_basis.clone(),
        Some(DcfAssumptions {
            growth,
            low_growth,
            high_growth,
            perpetual_growth,
            low_perpetual,
            high_perpetual,
            discount_rate,
            forecast_years,
            fcf_anchor: fcf,
            fcf_per_share,
            total_shares: shares,
            is_fallback_anchor,
            applicable,
            inapplicable_reason,
            applicability_signals,
            perpetual_clamped_by_negative_growth,
            configured_perpetual_growth,
            terminal_value_ratio,
            basis: fcf_basis,
        }),
    )
}

/// 格雷厄姆内在价值公式：V = EPS × (8.5 + 2g) × 4.4 / Y
/// g 为未来7-10年预期增长率，Y 为AAA企业债收益率基准
///
/// V74(2026-09-10): 返回 `Option<f64>`——EPS≤0 且近 5 年报无正 EPS 年度时
/// 返回 None（公式不适用），不再用 0 冒充估值。当期 EPS≤0 但历史存在正 EPS
/// 年报时，用正 EPS 均值归一化（周期底部锚定）。
///
/// P1-A(2026-09-11): EPS 一律经 `annualized_eps()` 取**年度口径**（中报/季报
/// 累计值还原为 TTM）；g 取数从 `profit_yoy` 改为 `revenue_yoy` 并允许负增长。
fn compute_graham_value(
    financials: &[FinancialReport],
    current_price: f64,
    config: Option<&ValuationConfig>,
) -> Option<f64> {
    if financials.is_empty() || current_price <= 0.0 {
        return None;
    }
    let cfg = config.copied().unwrap_or_default();
    let bond_yield = cfg.bond_yield();
    // 除零防护：bond_yield 来自模板配置，配成 0 会让估值变成 ±inf。
    if bond_yield <= 0.0 {
        return None;
    }

    let latest = &financials[0];
    let eps = annualized_eps(financials)?;
    // g 为未来 7-10 年预期增长率（小数形式，如 0.15）。P1-A 两处变更：
    //   ① `profit_yoy` → `revenue_yoy`：单期净利同比在季报口径下波动极大
    //      （002353 实测 profit_yoy = −3.6% vs revenue_yoy = +10.8%），
    //      长期增长预期用营收增速代理更稳定，且与 DCF 同源。
    //   ② clamp 下界 0.0 → −0.30：原实现把负增长抹平为 0，等于对衰退股
    //      默认「零增长」，反而**高估**其内在价值；上界 30% 保持不变。
    let g = latest
        .revenue_yoy
        .map(|y| (y / 100.0).clamp(-0.30, 0.30))
        .unwrap_or_else(|| cfg.default_growth());
    Some((eps * (8.5 + 2.0 * g) * 4.4 / bond_yield).max(0.0))
}

/// 巴菲特所有者收益（元）
/// 注意：vendor 返回的财务数据单位均为"元"，无需缩放
fn compute_owner_earnings(financials: &[FinancialReport]) -> Option<f64> {
    if financials.is_empty() {
        return None;
    }
    let f = &financials[0];
    // vendor 返回的财务数据单位均为"元"，无需缩放
    if let (Some(ocf), Some(capex)) = (f.operating_cash_flow, f.capital_expenditure) {
        Some((ocf - capex).max(0.0))
    } else if let Some(fcf) = f.free_cash_flow {
        Some(fcf.max(0.0))
    } else {
        let net = f.net_profit.unwrap_or(0.0);
        let debt_ratio = f.debt_ratio.unwrap_or(50.0);
        // debt_ratio 是百分比值，>60% 为高负债
        let factor = if debt_ratio > 60.0 {
            0.85
        } else if debt_ratio > 40.0 {
            0.90
        } else {
            0.95
        };
        Some((net * factor).max(0.0))
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Serenity 瓶颈筛选工具集（V58 补全）
// ═══════════════════════════════════════════════════════════════════════════
// 历史：seed_serenity.rs 注册了 7 个 ToolDef 但无实现，运行时全部失败。
// 修复：对接 astock-data 已有 API（get_financials/get_peers/get_quote 等），
// 输出契约严格对齐 bottleneck-calc.rhai / mapper_prompt 期望字段。

/// compute_industry_position：行业竞争地位分析
/// 输出契约（bottleneck-calc.rhai 期望）：
///   sector / competitive_position.{gross_margin_pct, roe_pct, debt_ratio_pct,
///   rnd_intensity, gm_rank_in_peers, total_peer_count} / capacity_indicators.signal
async fn compute_industry_position_impl(
    client: &crate::AStockClient,
    stock_code: &str,
) -> Result<serde_json::Value, String> {
    let financials = client.get_financials(stock_code).await.map_err(|e| e.to_string())?;
    let peers = client.get_peers(stock_code).await.map_err(|e| e.to_string())?;
    let sector_info = client.get_sector_info(stock_code).await.map_err(|e| e.to_string())?;

    let latest = financials.first();
    let gm_pct = latest.and_then(|f| f.gross_margin).unwrap_or(0.0);
    let roe_pct = latest.and_then(|f| f.roe).unwrap_or(0.0);
    let debt_ratio_pct = latest.and_then(|f| f.debt_ratio).unwrap_or(0.0);
    // R&D 强度无直接 API：用 (revenue - net_profit * 10) / revenue 近似
    // 保守估算：若 net_profit 为负或为 0，按营收 5% 默认值
    let rnd_intensity = latest
        .and_then(|f| {
            f.revenue.and_then(|rev| {
                if rev > 0.0 {
                    f.net_profit.map(|np| {
                        let rnd = (rev - np * 10.0).max(0.0);
                        (rnd / rev * 100.0).clamp(0.0, 30.0)
                    })
                } else {
                    None
                }
            })
        })
        .unwrap_or(5.0);
    // CapEx/折旧比：用 capex / (total_assets * 0.05) 近似（5% 折旧率）
    let capex_dep_ratio = latest
        .and_then(|f| {
            f.capital_expenditure.and_then(|capex| {
                f.total_assets.and_then(|ta| {
                    if ta > 0.0 {
                        Some((capex / (ta * 0.05)).clamp(0.0, 10.0))
                    } else {
                        None
                    }
                })
            })
        })
        .unwrap_or(0.0);
    let capex_signal = if capex_dep_ratio >= 3.0 {
        "积极扩产"
    } else if capex_dep_ratio >= 1.5 {
        "温和扩张"
    } else if capex_dep_ratio >= 1.0 {
        "维持投入"
    } else {
        "收缩投入"
    };

    // 在 peers 中按 ROE 排序计算个股排名（仅含有 ROE 数据的）
    let mut peer_roes: Vec<(String, f64)> =
        peers.iter().filter_map(|p| p.roe.map(|r| (p.stock_code.clone(), r))).collect();
    // 加入目标股本身
    if roe_pct > 0.0 || !peer_roes.is_empty() {
        peer_roes.push((stock_code.to_string(), roe_pct));
    }
    peer_roes.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let total_peer_count = peer_roes.len() as u32;
    let gm_rank_in_peers = peer_roes
        .iter()
        .position(|(code, _)| code == stock_code)
        .map(|i| (i + 1) as u32)
        .unwrap_or(0);

    let sector_full = match sector_info.as_ref() {
        None => "未知行业".to_string(),
        Some(si) if si.sub_sector.is_empty() => si.sector_name.clone(),
        Some(si) => format!("{}/{}", si.sector_name, si.sub_sector),
    };

    let result = json!({
        "stock_code": stock_code,
        "sector": sector_full,
        "competitive_position": {
            "gross_margin_pct": round2(gm_pct),
            "roe_pct": round2(roe_pct),
            "debt_ratio_pct": round2(debt_ratio_pct),
            "rnd_intensity": round2(rnd_intensity),
            "gm_rank_in_peers": gm_rank_in_peers,
            "total_peer_count": total_peer_count
        },
        "capacity_indicators": {
            "capex_dep_ratio": round2(capex_dep_ratio),
            "signal": capex_signal,
            "expansion_intensity": if capex_dep_ratio >= 3.0 { "high" }
                else if capex_dep_ratio >= 1.5 { "medium" }
                else { "low" }
        },
        "peer_count": peers.len(),
        "summary": format!(
            "行业:{sector_full} | 毛利率:{gm_pct:.1}% | ROE:{roe_pct:.1}% | 负债率:{debt_ratio_pct:.1}% | CapEx/折旧:{capex_dep_ratio:.2}({capex_signal})"
        )
    });
    Ok(result)
}

/// compute_bottleneck_signals：瓶颈信号计算
/// 输出契约（mapper_prompt 期望）：
///   inventory_turnover.{days_latest, days_yoy_change, signal}
///   gross_margin_trend.{latest, yoy_change, direction}
///   capex.{capex_dep_ratio, signal}
async fn compute_bottleneck_signals_impl(
    client: &crate::AStockClient,
    stock_code: &str,
) -> Result<serde_json::Value, String> {
    let financials = client.get_financials(stock_code).await.map_err(|e| e.to_string())?;

    if financials.is_empty() {
        return Ok(json!({
            "stock_code": stock_code,
            "inventory_turnover": null,
            "gross_margin_trend": null,
            "capex": null,
            "summary": "无财务数据"
        }));
    }

    let latest = &financials[0];
    let prev = financials.get(1);

    // 存货周转：用 revenue / total_assets 近似周转率，反推天数
    // 注意：astock-data 无存货字段，用资产周转率作为代理指标
    let asset_turnover_latest = latest
        .revenue
        .zip(latest.total_assets)
        .filter(|(_, ta)| *ta > 0.0)
        .map(|(rev, ta)| rev / ta);
    let asset_turnover_prev = prev
        .and_then(|p| p.revenue.zip(p.total_assets))
        .filter(|(_, ta)| *ta > 0.0)
        .map(|(rev, ta)| rev / ta);
    let days_latest =
        asset_turnover_latest.map(|t| if t > 0.0 { 365.0 / t } else { 0.0 }).unwrap_or(0.0);
    let days_yoy_change = match (asset_turnover_latest, asset_turnover_prev) {
        (Some(cl), Some(pv)) => {
            let dl = if cl > 0.0 { 365.0 / cl } else { 0.0 };
            let dp = if pv > 0.0 { 365.0 / pv } else { 0.0 };
            dl - dp
        },
        _ => 0.0,
    };
    let inventory_signal = if days_yoy_change > 30.0 {
        "accumulating"
    } else if days_yoy_change < -30.0 {
        "decelerating"
    } else {
        "stable"
    };

    // 毛利率趋势
    let gm_latest = latest.gross_margin.unwrap_or(0.0);
    let gm_yoy_change = prev.and_then(|p| p.gross_margin).map(|g| gm_latest - g).unwrap_or(0.0);
    let gm_direction = if gm_yoy_change > 1.0 {
        "expanding"
    } else if gm_yoy_change < -1.0 {
        "contracting"
    } else {
        "stable"
    };

    // CapEx/折旧比
    let capex_dep_ratio = latest
        .capital_expenditure
        .zip(latest.total_assets)
        .filter(|(_, ta)| *ta > 0.0)
        .map(|(capex, ta)| (capex / (ta * 0.05)).clamp(0.0, 10.0))
        .unwrap_or(0.0);
    let capex_signal = if capex_dep_ratio >= 3.0 {
        "积极扩产"
    } else if capex_dep_ratio >= 1.5 {
        "温和扩张"
    } else if capex_dep_ratio >= 1.0 {
        "维持投入"
    } else {
        "收缩投入"
    };

    let result = json!({
        "stock_code": stock_code,
        "inventory_turnover": {
            "days_latest": round2(days_latest),
            "days_yoy_change": round2(days_yoy_change),
            "signal": inventory_signal
        },
        "gross_margin_trend": {
            "latest": round2(gm_latest),
            "yoy_change": round2(gm_yoy_change),
            "direction": gm_direction
        },
        "capex": {
            "capex_dep_ratio": round2(capex_dep_ratio),
            "signal": capex_signal
        },
        "summary": format!(
            "存货周转天数:{days_latest:.0}天(同比{days_yoy_change:+.0}天,{inventory_signal}) | 毛利率:{gm_latest:.1}%({gm_direction}) | CapEx/折旧:{capex_dep_ratio:.2}({capex_signal})"
        )
    });
    Ok(result)
}

/// compute_attention_score：关注度评分
/// 输出契约（mapper_prompt attention_metrics 期望）：
///   attention_score / coverage_change_3m / search_heat / relative_volume / consensus_gap
async fn compute_attention_score_impl(
    client: &crate::AStockClient,
    stock_code: &str,
) -> Result<serde_json::Value, String> {
    // 并行拉取 4 类数据
    let (reports, news, quote, visits) = tokio::join!(
        client.get_research_reports(stock_code),
        client.get_news(stock_code, 30),
        client.get_quote(stock_code),
        client.get_institutional_visits(stock_code),
    );
    let reports = reports.map_err(|e| e.to_string()).unwrap_or_default();
    let news = news.map_err(|e| e.to_string()).unwrap_or_default();
    let quote = quote.map_err(|e| e.to_string()).ok();
    let visits = visits.map_err(|e| e.to_string()).unwrap_or_default();

    // 研报覆盖度（最近 90 天）
    let now = chrono::Utc::now();
    let cutoff_90d = now - chrono::Duration::days(90);
    let recent_reports: Vec<_> = reports
        .iter()
        .filter(|r| {
            chrono::DateTime::parse_from_rfc3339(&format!("{}T00:00:00Z", r.publish_date))
                .map(|dt| dt.with_timezone(&chrono::Utc) > cutoff_90d)
                .unwrap_or(false)
        })
        .collect();
    let research_count = recent_reports.len();
    let coverage_change_3m = if research_count == 0 {
        "无机构覆盖".to_string()
    } else if research_count < 3 {
        format!("近 3 月 {research_count} 篇研报（低覆盖）")
    } else if research_count < 8 {
        format!("近 3 月 {research_count} 篇研报（正常）")
    } else {
        format!("近 3 月 {research_count} 篇研报（高覆盖）")
    };

    // 新闻热度
    let news_count_30d = news.len();
    let search_heat = if news_count_30d < 10 {
        "冷门"
    } else if news_count_30d < 30 {
        "正常"
    } else {
        "热门"
    };

    // 换手率相对市场（A 股均值约 2%）
    let turnover = quote.as_ref().map(|q| q.turnover_rate).unwrap_or(0.0);
    let relative_volume = if turnover < 1.0 {
        format!("低于均值 {:.0}%", (2.0 - turnover).max(0.0) * 50.0)
    } else if turnover < 3.0 {
        "正常".to_string()
    } else {
        format!("高于均值 {:.0}%", (turnover - 2.0) * 50.0)
    };

    // 共识差：研报评级 vs 当前价
    // 简化：用研报数量 + 平均目标价 vs 当前价判断
    let avg_target = reports.iter().filter_map(|r| r.target_price).next(); // 取最新一篇的目标价
    let current_price = quote.as_ref().map(|q| q.price).unwrap_or(0.0);
    let consensus_gap = match (avg_target, current_price > 0.0) {
        (Some(target), true) => {
            let gap_pct = (target - current_price) / current_price * 100.0;
            if gap_pct > 30.0 {
                "明显低估"
            } else if gap_pct > 10.0 {
                "合理偏低"
            } else if gap_pct > -10.0 {
                "合理"
            } else {
                "高估"
            }
        },
        _ => "无研报共识",
    };

    // 机构调研数
    let visit_count = visits.len();

    // 综合关注度评分 0-100（越低越冷门）
    // 权重：研报覆盖 35% + 新闻热度 25% + 换手率 25% + 机构调研 15%
    let research_score = (research_count as f64 * 8.0).min(40.0); // 0-40
    let news_score = (news_count_30d as f64 * 1.5).min(30.0); // 0-30
    let turnover_score = (turnover * 10.0).min(20.0); // 0-20
    let visit_score = (visit_count as f64 * 3.0).min(10.0); // 0-10
    let attention_score =
        (research_score + news_score + turnover_score + visit_score).round() as u32;

    let result = json!({
        "stock_code": stock_code,
        "attention_score": attention_score.min(100),
        "coverage_change_3m": coverage_change_3m,
        "search_heat": search_heat,
        "relative_volume": relative_volume,
        "consensus_gap": consensus_gap,
        "components": {
            "research_coverage": research_count,
            "news_count_30d": news_count_30d,
            "visit_count": visit_count,
            "turnover_rate_pct": round2(turnover),
            "current_price": round2(current_price),
            "avg_target_price": avg_target.map(round2)
        },
        "summary": format!(
            "关注度评分:{}/100 | 研报:{research_count}篇 | 新闻:{news_count_30d}条 | 换手率:{turnover:.2}% | 机构调研:{visit_count}次",
            attention_score.min(100)
        )
    });
    Ok(result)
}

/// check_exit_signals：退出信号检查
/// 输出契约（mapper_prompt exit_signals 期望）：
///   technology_disruption_risk / capacity_oversupply_risk / new_entrant_risk
///   demand_slowdown_risk / overall_exit_urgency
async fn check_exit_signals_impl(
    client: &crate::AStockClient,
    stock_code: &str,
    entry_price: Option<f64>,
    stop_loss_price: Option<f64>,
) -> Result<serde_json::Value, String> {
    let (financials, news, quote) = tokio::join!(
        client.get_financials(stock_code),
        client.get_news(stock_code, 30),
        client.get_quote(stock_code),
    );
    let financials = financials.map_err(|e| e.to_string()).unwrap_or_default();
    let news = news.map_err(|e| e.to_string()).unwrap_or_default();
    let quote = quote.map_err(|e| e.to_string()).ok();

    let latest = financials.first();
    let prev = financials.get(1);

    // 1. 技术替代风险：扫描近期新闻关键词
    let disruption_keywords = ["替代", "颠覆", "新技术", "突破", "新一代", "替代品", "颠覆性"];
    let disruption_hits = news
        .iter()
        .filter(|n| {
            disruption_keywords.iter().any(|k| n.title.contains(k) || n.summary.contains(k))
        })
        .count();
    let technology_disruption_risk = if disruption_hits >= 3 {
        "高 - 近期有技术替代报道"
    } else if disruption_hits >= 1 {
        "中 - 个别替代相关新闻"
    } else {
        "低 - 暂无技术替代报道"
    };

    // 2. 产能过剩风险：存货周转天数变化 + CapEx 强度
    let asset_turnover_change = match (latest, prev) {
        (Some(l), Some(p)) => {
            let lt =
                l.revenue.zip(l.total_assets).filter(|(_, ta)| *ta > 0.0).map(|(r, ta)| r / ta);
            let pt =
                p.revenue.zip(p.total_assets).filter(|(_, ta)| *ta > 0.0).map(|(r, ta)| r / ta);
            match (lt, pt) {
                (Some(l), Some(p)) => Some(l - p),
                _ => None,
            }
        },
        _ => None,
    };
    let capex_dep_ratio = latest
        .and_then(|l| {
            l.capital_expenditure
                .zip(l.total_assets)
                .filter(|(_, ta)| *ta > 0.0)
                .map(|(capex, ta)| (capex / (ta * 0.05)).clamp(0.0, 10.0))
        })
        .unwrap_or(0.0);
    let capacity_oversupply_risk = match asset_turnover_change {
        Some(change) if change < -0.1 && capex_dep_ratio > 2.0 => "高 - 周转率下滑且 CapEx 高强度",
        Some(change) if change < -0.1 => "中 - 周转率下滑",
        Some(_) if capex_dep_ratio > 3.0 => "中 - CapEx 强度偏高，关注产能释放",
        _ => "低 - 周转率稳定",
    };

    // 3. 新进入者风险：壁垒评估（ROE + 毛利率）
    let roe = latest.and_then(|f| f.roe).unwrap_or(0.0);
    let gm = latest.and_then(|f| f.gross_margin).unwrap_or(0.0);
    let debt = latest.and_then(|f| f.debt_ratio).unwrap_or(50.0);
    let new_entrant_risk = if gm > 50.0 && roe > 15.0 && debt < 40.0 {
        "低 - 高毛利+高ROE+低负债，行业壁垒高"
    } else if gm > 30.0 || roe > 10.0 {
        "中 - 中等壁垒"
    } else {
        "高 - 低毛利/低ROE，壁垒薄弱"
    };

    // 4. 需求放缓风险：营收同比
    let revenue_yoy = latest.and_then(|f| f.revenue_yoy).unwrap_or(0.0);
    let profit_yoy = latest.and_then(|f| f.profit_yoy).unwrap_or(0.0);
    let demand_slowdown_risk = if revenue_yoy < -10.0 || profit_yoy < -20.0 {
        "高 - 营收或利润显著下滑"
    } else if revenue_yoy < 0.0 || profit_yoy < 0.0 {
        "中 - 营收或利润负增长"
    } else if revenue_yoy < 10.0 {
        "低 - 增速放缓但未负增长"
    } else {
        "低 - 营收稳健增长"
    };

    // 5. 价格止损触发
    let current_price = quote.as_ref().map(|q| q.price).unwrap_or(0.0);
    let price_stop_triggered = match (entry_price, stop_loss_price, current_price > 0.0) {
        (Some(entry), Some(stop), true) => {
            // 触发条件：当前价 <= 止损价 或 当前价 < 买入价 * 0.85（默认 -15%）
            let dynamic_stop = if stop > 0.0 { stop } else { entry * 0.85 };
            current_price <= dynamic_stop
        },
        _ => false,
    };

    // 综合退出紧迫度
    let risk_count = [
        technology_disruption_risk.starts_with("高"),
        capacity_oversupply_risk.starts_with("高"),
        new_entrant_risk.starts_with("高"),
        demand_slowdown_risk.starts_with("高"),
    ]
    .iter()
    .filter(|&&x| x)
    .count();
    let medium_count = [
        technology_disruption_risk.starts_with("中"),
        capacity_oversupply_risk.starts_with("中"),
        new_entrant_risk.starts_with("中"),
        demand_slowdown_risk.starts_with("中"),
    ]
    .iter()
    .filter(|&&x| x)
    .count();
    let overall_exit_urgency = if price_stop_triggered || risk_count >= 2 {
        "exit_now"
    } else if risk_count >= 1 || medium_count >= 2 {
        "caution"
    } else if medium_count >= 1 {
        "watch"
    } else {
        "no_urgency"
    };

    let result = json!({
        "stock_code": stock_code,
        "technology_disruption_risk": technology_disruption_risk,
        "capacity_oversupply_risk": capacity_oversupply_risk,
        "new_entrant_risk": new_entrant_risk,
        "demand_slowdown_risk": demand_slowdown_risk,
        "overall_exit_urgency": overall_exit_urgency,
        "price_check": {
            "current_price": round2(current_price),
            "entry_price": entry_price.map(round2),
            "stop_loss_price": stop_loss_price.map(round2),
            "stop_triggered": price_stop_triggered
        },
        "summary": format!(
            "退出紧迫度:{overall_exit_urgency} | 高风险{risk_count}项, 中风险{medium_count}项"
        )
    });
    Ok(result)
}

/// verify_catalysts：催化剂验证
async fn verify_catalysts_impl(
    client: &crate::AStockClient,
    stock_code: &str,
    catalysts: &[String],
) -> Result<serde_json::Value, String> {
    let news = client.get_news(stock_code, 50).await.map_err(|e| e.to_string())?;

    let verified: Vec<serde_json::Value> = catalysts
        .iter()
        .map(|c| {
            // 从催化剂描述中提取关键词（按空格/逗号分割）
            let keywords: Vec<&str> =
                c.split([' ', ',', '，', '/']).filter(|s| s.chars().count() >= 2).collect();
            // 在新闻标题中匹配关键词
            let hits: Vec<&str> = news
                .iter()
                .filter(|n| keywords.iter().any(|k| n.title.contains(k) || n.summary.contains(k)))
                .map(|n| n.title.as_str())
                .collect();
            let hit_count = hits.len();
            let status = if hit_count >= 3 {
                "confirmed"
            } else if hit_count >= 1 {
                "partial"
            } else {
                "unverified"
            };
            let confidence = if hit_count >= 3 {
                85
            } else if hit_count >= 1 {
                60
            } else {
                30
            };
            json!({
                "catalyst": c,
                "status": status,
                "evidence": hits.iter().take(3).collect::<Vec<_>>(),
                "hit_count": hit_count,
                "confidence": confidence
            })
        })
        .collect();
    let unverified_count =
        verified.iter().filter(|v| v["status"].as_str() == Some("unverified")).count() as u32;
    let total = verified.len() as u32;
    let confirmed = total - unverified_count;

    let result = json!({
        "stock_code": stock_code,
        "verified": verified,
        "total_count": total,
        "confirmed_count": confirmed,
        "unverified_count": unverified_count,
        "summary": format!("{confirmed}/{total} 催化剂已验证（{unverified_count} 未验证）")
    });
    Ok(result)
}

/// compute_serenity_performance：Serenity 候选推荐后表现
async fn compute_serenity_performance_impl(
    client: &crate::AStockClient,
    stock_code: &str,
    recommend_date: &str,
) -> Result<serde_json::Value, String> {
    if recommend_date.is_empty() {
        return Err("compute_serenity_performance 缺少 recommend_date 参数".to_string());
    }
    let recommend_dt = chrono::NaiveDate::parse_from_str(recommend_date, "%Y-%m-%d")
        .map_err(|e| format!("recommend_date 格式错误（应为 YYYY-MM-DD）: {e}"))?;
    let today = chrono::Utc::now().date_naive();
    let holding_days = (today - recommend_dt).num_days();

    if holding_days <= 0 {
        return Ok(json!({
            "stock_code": stock_code,
            "recommend_date": recommend_date,
            "return_pct": 0.0,
            "outperform_pct": 0.0,
            "holding_days": 0,
            "hit_target": false,
            "hit_stop": false,
            "status": "future_date"
        }));
    }

    // 拉取推荐日至今的 K 线（日 K，足够覆盖 1 年内）
    let limit = (holding_days as u32 + 30).min(500);
    let klines = client.get_klines(stock_code, "daily", limit).await.map_err(|e| e.to_string())?;
    // 找到推荐日附近的 K 线
    let baseline = klines.iter().find(|k| k.date.starts_with(recommend_date));
    let latest = klines.last();
    let return_pct = match (baseline, latest) {
        (Some(base), Some(last)) if base.close > 0.0 => {
            (last.close - base.close) / base.close * 100.0
        },
        _ => 0.0,
    };

    // 大盘基准（上证指数）
    let index_quotes = client.get_index_quotes().await.map_err(|e| e.to_string())?;
    let sh_index = index_quotes.iter().find(|q| q.code.starts_with("000001"));
    let index_change_pct = sh_index.map(|q| q.change_pct).unwrap_or(0.0);
    // outperform_pct = return_pct - 当日大盘涨跌幅
    // 注：这里用今日大盘涨跌幅作为近似（推荐日至今的累计需要历史 K 线，
    //     简化处理：仅用今日大盘涨跌幅作为相对参考）
    let outperform_pct = return_pct - index_change_pct;

    // 止盈止损触发（默认 +30% 止盈，-15% 止损）
    let hit_target = return_pct >= 30.0;
    let hit_stop = return_pct <= -15.0;
    let status = if hit_target {
        "hit_target"
    } else if hit_stop {
        "hit_stop"
    } else if holding_days > 90 {
        "expired"
    } else {
        "active"
    };

    let result = json!({
        "stock_code": stock_code,
        "recommend_date": recommend_date,
        "return_pct": round2(return_pct),
        "outperform_pct": round2(outperform_pct),
        "holding_days": holding_days,
        "hit_target": hit_target,
        "hit_stop": hit_stop,
        "status": status,
        "baseline_price": baseline.map(|b| round2(b.close)),
        "latest_price": latest.map(|l| round2(l.close)),
        "summary": format!(
            "持有{holding_days}天 | 涨幅:{return_pct:.2}% | 超额:{outperform_pct:+.2}% | 状态:{status}"
        )
    });
    Ok(result)
}

/// optimize_attention_weights：基于历史样本调优关注度权重
/// 纯算法实现，不调 API
fn optimize_attention_weights_impl(samples: &Vec<serde_json::Value>) -> serde_json::Value {
    let sample_count = samples.len();
    if sample_count == 0 {
        return json!({
            "weights": {
                "coverage_weight": 0.35,
                "search_weight": 0.25,
                "volume_weight": 0.25,
                "gap_weight": 0.15
            },
            "expected_accuracy": 0.5,
            "sample_count": 0,
            "summary": "无样本输入，返回默认权重"
        });
    }

    // 简化策略：根据 attention_score 与 actual_return_pct 的相关性反推权重
    // 高 attention_score 应对应低 actual_return（Serenity 假说：低关注度 → 高弹性）
    // 用样本统计验证假说强度
    let mut high_attn_returns: Vec<f64> = Vec::new();
    let mut low_attn_returns: Vec<f64> = Vec::new();
    let mut total_score = 0.0;
    let mut total_return = 0.0;
    for s in samples {
        let attn = s["attention_score"].as_f64().unwrap_or(50.0);
        let ret = s["actual_return_pct"].as_f64().unwrap_or(0.0);
        total_score += attn;
        total_return += ret;
        if attn >= 50.0 {
            high_attn_returns.push(ret);
        } else {
            low_attn_returns.push(ret);
        }
    }
    let _avg_attn = total_score / sample_count as f64;
    let avg_return = total_return / sample_count as f64;
    let low_avg = if !low_attn_returns.is_empty() {
        low_attn_returns.iter().sum::<f64>() / low_attn_returns.len() as f64
    } else {
        0.0
    };
    let high_avg = if !high_attn_returns.is_empty() {
        high_attn_returns.iter().sum::<f64>() / high_attn_returns.len() as f64
    } else {
        0.0
    };

    // 假说验证：低关注度组平均收益是否高于高关注度组
    let hypothesis_valid = low_avg > high_avg;
    let spread = (low_avg - high_avg).abs();
    // 假说越显著，coverage_weight 越大
    let coverage_weight = if hypothesis_valid && spread > 5.0 {
        0.45
    } else if hypothesis_valid {
        0.35
    } else {
        0.25
    };
    let search_weight = 0.20;
    let volume_weight = 0.20;
    let gap_weight = 1.0 - coverage_weight - search_weight - volume_weight;

    // 期望准确率：用样本均值偏离度近似（粗略指标）
    let return_std = {
        let mean = avg_return;
        let var = samples
            .iter()
            .filter_map(|s| s["actual_return_pct"].as_f64())
            .map(|r| (r - mean).powi(2))
            .sum::<f64>()
            / sample_count as f64;
        var.sqrt()
    };
    let expected_accuracy = if return_std > 0.0 {
        (1.0 / (1.0 + return_std / 20.0)).clamp(0.3, 0.9)
    } else {
        0.5
    };

    json!({
        "weights": {
            "coverage_weight": round2(coverage_weight),
            "search_weight": round2(search_weight),
            "volume_weight": round2(volume_weight),
            "gap_weight": round2(gap_weight)
        },
        "expected_accuracy": round2(expected_accuracy),
        "sample_count": sample_count,
        "hypothesis_validation": {
            "low_attention_avg_return_pct": round2(low_avg),
            "high_attention_avg_return_pct": round2(high_avg),
            "hypothesis_valid": hypothesis_valid,
            "spread_pct": round2(spread)
        },
        "summary": format!(
            "样本:{sample_count} | 低关注度组均值收益:{low_avg:.2}% | 高关注度组:{high_avg:.2}% | 假说验证:{hypothesis_valid}"
        )
    })
}

#[cfg(test)]
mod valuation_tests {
    use super::*;

    /// 构造财报: report_date 含 "-12-31" 视为年报（与 normalized_*_helper 口径一致）
    fn report(date: &str, np: Option<f64>, eps: Option<f64>) -> FinancialReport {
        FinancialReport {
            stock_code: "600000".into(),
            report_date: date.into(),
            revenue: None,
            net_profit: np,
            eps,
            bps: None,
            roe: None,
            debt_ratio: None,
            gross_margin: None,
            net_margin: None,
            revenue_yoy: None,
            profit_yoy: None,
            total_assets: None,
            operating_cash_flow: None,
            capital_expenditure: None,
            free_cash_flow: None,
            current_ratio: None,
            quick_ratio: None,
            goodwill: None,
            accounts_receivable: None,
            estimated: Some(false),
        }
    }

    fn shares_of(shares: f64) -> Option<f64> {
        Some(shares)
    }

    /// `report` 的 ROE 变体
    fn report_roe(date: &str, roe: f64) -> FinancialReport {
        let mut r = report(date, None, None);
        r.roe = Some(roe);
        r
    }

    /// 2026-09-14 601166 实证：中报 ROE 是**年内累计值**（半年），
    /// 与同一份报告里已年化的 PE 口径不一致，必须先还原再参与任何判定。
    ///
    /// 未还原时的后果链（全部逐条在 DB 中可查）：
    /// `roe = 4.8` → `a-fundamentals` bear_points 首条「ROE 严重下滑至 4.8%」→
    /// `risk-agg`/`risk-neu`/`risk-con`/`research-mgr` 四份报告复述为
    /// 「严重偏离行业均值 10-12%」→ `health_score` 掉档 → 决策保守化。
    /// 还原后 ≈ 9.8%，落在银行业正常区间下沿，结论方向随之改变。
    #[test]
    fn roe_ttm_restores_interim_cumulative_to_annual() {
        // 601166 形态：2025FY 9.6 + 2026H1 4.8 − 2025H1 5.0 = 9.4
        let financials = vec![
            report_roe("2026-06-30", 4.8),
            report_roe("2025-12-31", 9.6),
            report_roe("2025-06-30", 5.0),
        ];
        let roe = annualized_roe(&financials).expect("应还原出年度口径 ROE");
        assert!((roe - 9.4).abs() < 1e-9, "TTM 还原应为 9.4，实际 {roe}");
        // 关键边界：`portfolio-mgr.rhai` 的 `is_high` 含 `roe < RISK_ROE_HIGH(5.0)`。
        // 还原后仍 ≤ 5.0 ⇒ 本次修复对该判定无影响，必须暴露出来而不是静默"通过"。
        assert!(roe > 5.0, "还原后必须越过 RISK_ROE_HIGH(5.0) 阈值，否则修复对该风险判定无意义");
    }

    /// 年报口径本身就是年度值 —— 原样使用，不做任何缩放
    #[test]
    fn roe_annual_report_used_verbatim() {
        let financials = vec![report_roe("2025-12-31", 9.6), report_roe("2024-12-31", 8.0)];
        assert_eq!(annualized_roe(&financials), Some(9.6));
    }

    /// 缺上年同期 → 按报告期月份数粗年化（半年 ×2）
    #[test]
    fn roe_falls_back_to_period_scaling_when_prior_period_missing() {
        let financials = vec![report_roe("2026-06-30", 4.8), report_roe("2025-12-31", 9.6)];
        let roe = annualized_roe(&financials).expect("应粗年化");
        assert!((roe - 9.6).abs() < 1e-9, "半年值应 ×2 = 9.6，实际 {roe}");
    }

    /// 无任何可用历史 → None。调用方据此**保留原值**，不得以 0 或猜测值冒充。
    #[test]
    fn roe_returns_none_without_history() {
        let financials = vec![report_roe("2026-06-30", 4.8)];
        assert_eq!(annualized_roe(&financials), None);
    }

    /// V74: 当期亏损但近5年报有正净利 → 归一化锚定，DCF 不再退化 0
    #[test]
    fn dcf_normalizes_when_latest_loss_but_history_positive() {
        let financials = vec![
            report("2026-06-30", Some(-8.38e8), Some(-0.5)),
            report("2025-12-31", Some(5.0e8), Some(0.30)),
            report("2024-12-31", Some(8.0e8), Some(0.48)),
            report("2023-12-31", Some(6.5e8), Some(0.39)),
            report("2022-12-31", Some(7.2e8), Some(0.43)),
        ];
        let (tiers, note, a) = compute_dcf(&financials, shares_of(20.0e8), 7.91, None);
        let (low, mid, high) = tiers.expect("归一化锚定后 DCF 应可用");
        assert!(low > 0.0 && mid > 0.0 && high > 0.0);
        assert!(low < mid && mid < high, "三档应单调: {low} < {mid} < {high}");
        assert!(note.contains("归一化"), "口径说明应标注归一化: {note}");
        // P0-I(2026-09-12): 本分支 == f5 置信度衰减的**唯一触发条件**。
        // `is_fallback_anchor` 若在 fallback 路径下取不到 true，
        // `portfolio-mgr.rhai` 的 `f5_fallback_decay` 将恒为 1.0 —— 衰减静默失效且不报错。
        let a = a.expect("应回传参数快照");
        assert!(
            a.is_fallback_anchor,
            "走了「近5年报正净利均值×0.90」fallback 却未标记 is_fallback_anchor ⇒ f5 衰减失效"
        );
    }

    /// V74: 持续亏损（近5年报无正净利）→ None + 原因，不再输出 (0,0,0) 冒充估值
    #[test]
    fn dcf_returns_none_for_persistent_loss() {
        let financials = vec![
            report("2026-06-30", Some(-8.38e8), Some(-0.5)),
            report("2025-12-31", Some(-3.0e8), Some(-0.18)),
            report("2024-12-31", Some(-1.5e8), Some(-0.09)),
        ];
        let (tiers, note, _) = compute_dcf(&financials, shares_of(20.0e8), 7.91, None);
        assert!(tiers.is_none(), "持续亏损应返回 None");
        assert!(note.contains("不适用"), "原因说明: {note}");
    }

    /// V74: 当期盈利（直接 FCF 口径）路径不受影响
    #[test]
    fn dcf_direct_fcf_path_unchanged() {
        let mut financials = vec![report("2025-12-31", Some(10.0e8), Some(0.6))];
        financials[0].free_cash_flow = Some(6.0e8);
        let (tiers, note, a) = compute_dcf(&financials, shares_of(10.0e8), 15.0, None);
        let (_, mid, _) = tiers.expect("正常 FCF 应可用");
        assert!(mid > 0.0);
        assert_eq!(note, "当期FCF");
        // P0-I(2026-09-12): **反向断言** —— 当期真实 FCF 分支不得被标为 fallback。
        // 若实现退化成「一律置 true」，真实锚定的估值信号也会被无谓腰斩，
        // 这是与上一条断言方向相反、必须同时存在的护栏。
        let a = a.expect("应回传参数快照");
        assert!(!a.is_fallback_anchor, "当期真实 FCF 被误标为 fallback ⇒ 真实估值信号被无谓衰减");
    }

    /// V74: 股本缺失 → None，不再输出 (0,0,0)
    #[test]
    fn dcf_none_when_shares_missing() {
        let financials = vec![report("2025-12-31", Some(5.0e8), Some(0.3))];
        let (tiers, note, _) = compute_dcf(&financials, None, 7.91, None);
        assert!(tiers.is_none());
        assert!(note.contains("股本"));
    }

    /// 2026-09-12：`dcf.assumptions` 必须回传**实际生效参数**，使估值可对账。
    ///
    /// 该不变量正是「`603353` 的 `mid = 2.18` 为何复现不出」的根因 —— 旧输出只落
    /// 三档数值，而三档给出 3 个方程、待定量有 5 个（growth / perpetual / discount /
    /// anchor / shares）⇒ **欠定**，实测三组差异很大的锚定假设能同时「命中」同一份
    /// 存档值（误差 ~0.5%）。落参数后对账即唯一解（见 AUDIT §7.4.6）。
    ///
    /// ✅ 2026-09-12（PLAN P0-F 方案 A）此偏置**已修**：`revenue_yoy = −6.05%` 不再被抬成
    /// `+2%`，`a.growth` 直接等于 `−0.0605`。下方断言已按修复后语义更新。
    #[test]
    fn dcf_assumptions_report_effective_params() {
        let mut financials = vec![report("2025-12-31", Some(5.0e8), Some(0.3))];
        financials[0].revenue_yoy = Some(-6.05);
        let (tiers, _, a) = compute_dcf(&financials, shares_of(10.0e8), 15.0, None);
        let (low, mid, high) = tiers.expect("有正净利 → 归一化锚定可用");
        let a = a.expect("应回传参数快照");

        // 不变量: growth = revenue_yoy/100 再统一 clamp 到 [min_growth, max_growth]
        // 修复前为 MIN_GROWTH(+0.02)（负增长被抬成正增长），现原样保留 −6.05%
        assert!(
            (a.growth - (-0.0605)).abs() < 1e-9,
            "负增长应原样保留 −6.05%，实际 {}（若为 +0.02 说明 P0-F 被回退）",
            a.growth
        );
        assert_eq!(a.discount_rate, DISCOUNT_RATE);
        // ⚠️ 2026-09-14 语义变更：本样本 `growth = −6.05%` 为负 ⇒ 触发
        //   「永续增长率符号一致性约束」，生效值被压到 0，**不再等于配置值**。
        //   配置原值不丢弃，落进 `configured_perpetual_growth` 供对账。
        //   （断言写在两处而非只写一处：`perpetual_growth == 0` 与
        //   `configured == PERPETUAL_GROWTH` 必须同时成立，否则说明约束被误伤成
        //   「无条件清零」或「配置值被吞」。）
        assert_eq!(
            a.perpetual_growth, 0.0,
            "预测期负增长时永续增长率必须被压到 0（否则=「5 年萎缩后永久正增长」）"
        );
        assert_eq!(
            a.configured_perpetual_growth, PERPETUAL_GROWTH,
            "配置原值必须原样保留以供对账，不得被静默丢弃"
        );
        assert!(
            a.perpetual_clamped_by_negative_growth,
            "被约束压回却未置标记 ⇒ 下游/面板无法区分「配置就是 0」与「被强制压回」"
        );
        assert_eq!(a.forecast_years, FORECAST_YEARS);
        assert_eq!(a.total_shares, 10.0e8);
        // 锚定值 = 近 5 年报正净利均值 × 0.90
        assert!(
            (a.fcf_anchor - 5.0e8 * 0.9).abs() < 1.0,
            "锚定值应为 4.5e8，实际 {}",
            a.fcf_anchor
        );
        // 档位序：快照里的 low/mid/high 参数必须与三档数值同序。
        // 注意负增长下 `low_growth` 的**数值更小**（−8.47% vs −6.05%），语义是「更悲观」。
        assert!(low < mid && mid < high, "档位乱序: {low} {mid} {high}");
        assert!(
            a.low_growth < a.growth && a.growth < a.high_growth,
            "负增长档位序错: low={} mid={} high={}",
            a.low_growth,
            a.growth,
            a.high_growth
        );
        assert!((a.low_growth - (-0.0605 * 1.4)).abs() < 1e-9);
        assert!((a.high_growth - (-0.0605 * 0.6)).abs() < 1e-9);
    }

    /// 档位序不变量 `low ≤ mid ≤ high` 必须对**任意符号**的 `growth` 成立。
    ///
    /// 回归对象：旧实现 `low_growth = growth × 0.6` / `high_growth = growth × 1.5`
    /// 是无条件缩放，`growth < 0` 时 `×0.6` 反而**变大**（−20% → −12%）⇒ 保守档比
    /// 中性档乐观，`low > mid` 且 `high < mid` 的档位乱序（静默无日志）。
    /// 这正是旧代码把 `MIN_GROWTH` 定在 `+0.02` 的隐性动机 —— 用「不允许负增长」
    /// 掩盖缩放方向错误，代价是**所有衰退股被系统性高估**。
    #[test]
    fn dcf_tier_ordering_holds_for_negative_growth() {
        // 覆盖：温和衰退 / 重度衰退（触 MIN_GROWTH 下界）/ 高增长（触 MAX 上界）
        for yoy in [-6.05_f64, -20.0, -80.0, 5.0, 12.0, 60.0] {
            let mut financials = vec![report("2025-12-31", Some(5.0e8), Some(0.3))];
            financials[0].revenue_yoy = Some(yoy);
            let (tiers, _, a) = compute_dcf(&financials, shares_of(10.0e8), 15.0, None);
            let (low, mid, high) = tiers.expect("有正净利 → 归一化锚定可用");
            let a = a.expect("应回传参数快照");

            assert!(
                low <= mid && mid <= high,
                "yoy={yoy}% 档位乱序: low={low} mid={mid} high={high}"
            );
            assert!(
                a.low_growth <= a.growth && a.growth <= a.high_growth,
                "yoy={yoy}% 参数档位乱序: low={} mid={} high={}",
                a.low_growth,
                a.growth,
                a.high_growth
            );
            // 统一 clamp：growth 必须落在 [min_growth, max_growth] 内（含缺省分支）
            assert!(
                a.growth >= MIN_GROWTH && a.growth <= MAX_GROWTH,
                "yoy={yoy}% growth={} 越界",
                a.growth
            );
            assert!(a.low_perpetual >= 0.0, "永续增长率不得为负");
            assert!(a.high_perpetual <= MAX_PERPETUAL_GROWTH);
        }
    }

    /// `growth` 缺省分支（`revenue_yoy = None`）也必须经过 clamp —— 用户把扁平参数
    /// `dcf_growth_rate` 配成 >30% 时，旧实现 `high_growth` 会被 clamp 回 `MAX_GROWTH`
    /// 而 `mid` 不 clamp ⇒ `high < mid` 的档位乱序（静默）。
    #[test]
    fn dcf_default_growth_branch_is_clamped() {
        let over = serde_json::json!({ "dcf_growth_rate": 80.0 });
        let cfg = ValuationConfig::from_flat_arguments(&over);
        let financials = vec![report("2025-12-31", Some(5.0e8), Some(0.3))]; // revenue_yoy = None
        let (tiers, _, a) = compute_dcf(&financials, shares_of(10.0e8), 15.0, cfg.as_ref());
        let (low, mid, high) = tiers.expect("有正净利 → 归一化锚定可用");
        let a = a.expect("应回传参数快照");

        assert_eq!(a.growth, MAX_GROWTH, "缺省分支未 clamp: {}", a.growth);
        assert!(low <= mid && mid <= high, "档位乱序: {low} {mid} {high}");
    }

    // ── C2 路径 Z(2026-09-12): 扁平参数接线 ──

    /// 百分数口径 → 小数，单点换算（PLAN 决策 D1）
    #[test]
    fn flat_args_percent_converts_to_decimal() {
        let args = serde_json::json!({
            "dcf_growth_rate": 12.0,
            "dcf_perpetual_rate": 4.0,
            "dcf_discount_rate": 8.5,
        });
        let cfg = ValuationConfig::from_flat_arguments(&args).expect("三项均有效应返回 Some");
        assert_eq!(cfg.default_growth(), 0.12);
        assert_eq!(cfg.perpetual_growth(), 0.04);
        assert_eq!(cfg.discount_rate(), 0.085);
        // 未提供的项回退模块常量
        assert_eq!(cfg.min_growth(), MIN_GROWTH);
        assert_eq!(cfg.bond_yield(), 4.4);
    }

    /// 越界值必须被忽略（不可成为「折现率 850%」的来源）；单边有效时另一边的越界项
    /// 自行回退，不得连带丢弃有效项
    #[test]
    fn flat_args_ignores_out_of_range() {
        // 全部越界 → None（回退常量默认）
        let bad = serde_json::json!({
            "dcf_growth_rate": 850.0,
            "dcf_perpetual_rate": -1.0,
            "dcf_discount_rate": 0.0,
        });
        assert!(ValuationConfig::from_flat_arguments(&bad).is_none());

        // 单边有效 + 单边越界 → 取有效项，越界项回退
        let mixed = serde_json::json!({ "dcf_discount_rate": 6.0, "dcf_growth_rate": 999.0 });
        let cfg = ValuationConfig::from_flat_arguments(&mixed).expect("应保留有效项");
        assert_eq!(cfg.discount_rate(), 0.06);
        assert_eq!(cfg.default_growth(), DEFAULT_GROWTH);
    }

    /// 无扁平参数（模板未接线 / 旧模板）→ None，保证零回归
    #[test]
    fn flat_args_empty_is_none() {
        let empty = serde_json::json!({ "stock_code": "600519" });
        assert!(ValuationConfig::from_flat_arguments(&empty).is_none());
    }

    /// 集成：扁平参数降低折现率必须**真实抬高** DCF 中性档（证明接线生效而非被吞）
    ///
    /// ⚠️ 覆盖值必须与常量 `DISCOUNT_RATE` **不同**，否则 base 与 tuned 落在同一参数上，
    /// 断言恒等成立却毫无区分度。本测试初版取 `8.5`，恰与路径 Y 对齐后的常量
    /// `DISCOUNT_RATE = 0.085` 重合，导致 `base == tuned` 的假阴性。
    ///
    /// 只覆盖折现率、不动永续增长率：p 与 d 对终值的作用方向相反
    /// （`TV = FCF×(1+p)/(d−p)`，p↓ 使分子↓且分母↑），同时改会掩盖单一变量信号。
    #[test]
    fn dcf_uses_flat_args_discount_rate() {
        let mut financials = vec![report("2025-12-31", Some(10.0e8), Some(0.6))];
        financials[0].free_cash_flow = Some(6.0e8);
        financials[0].revenue_yoy = Some(10.8);

        let (base, _, _) = compute_dcf(&financials, shares_of(10.0e8), 15.0, None);
        let base_mid = base.expect("基线应可用").1;

        // 6% 明显低于常量 8.5% → 折现率下降，中性档必须抬高
        let args = serde_json::json!({ "dcf_discount_rate": 6.0 });
        let cfg = ValuationConfig::from_flat_arguments(&args).expect("扁平参数应可解析");
        let (tuned, _, _) = compute_dcf(&financials, shares_of(10.0e8), 15.0, Some(&cfg));
        let tuned_mid = tuned.expect("调参后应可用").1;

        assert!(
            tuned_mid > base_mid,
            "折现率 {}%→6% 应抬高中性档: base={base_mid} tuned={tuned_mid}",
            DISCOUNT_RATE * 100.0
        );
    }

    /// 不变量：乐观档永续增长率上限必须低于折现率，否则终值走 `(d−p).max(0.001)`
    /// 兜底分支导致失真。
    ///
    /// 用 `const` block 求值：两个操作数都是编译期常量，普通 `assert!` 会被
    /// `clippy::assertions_on_constants` 判为常量断言。放进 const block 后违规在
    /// **编译期**即失败，比运行期测试更早暴露。
    #[test]
    fn max_perpetual_growth_stays_below_discount_rate() {
        const { assert!(MAX_PERPETUAL_GROWTH < DISCOUNT_RATE) };
    }

    /// V74: 当期 EPS≤0 但历史有正 EPS → 归一化，格雷厄姆值不再恒 0
    #[test]
    fn graham_normalizes_when_latest_eps_negative() {
        let financials = vec![
            report("2026-06-30", Some(-8.38e8), Some(-0.5)),
            report("2025-12-31", Some(5.0e8), Some(0.30)),
            report("2024-12-31", Some(8.0e8), Some(0.48)),
        ];
        let v = compute_graham_value(&financials, 7.91, None).expect("归一化后格雷厄姆值应可用");
        assert!(v > 0.0);
        // 最新为 H1 累计且缺上年同期 → TTM 不可得 → 回退近 5 年报正 EPS 均值 = 0.39
        // g 缺省用 DEFAULT_GROWTH（P1-A 后与 DCF 同源）→ v = 0.39 × (8.5 + 0.16)
        let expected = 0.39 * (8.5 + 2.0 * DEFAULT_GROWTH);
        assert!((v - expected).abs() < 1e-6, "v={v}, expected={expected}");
    }

    /// P1-A: 中报累计 EPS 必须还原为 TTM，不得直接当年度值用
    /// （002353 实测：中报 EPS 1.18 让 graham 从 22.10 掉到 10.03，错误值还进了 LLM 看空论据）
    #[test]
    fn graham_annualizes_interim_eps_to_ttm() {
        let financials = vec![
            report("2026-06-30", Some(1.18e9), Some(1.18)), // 本期 H1 累计
            report("2025-12-31", Some(2.64e9), Some(2.64)), // 上年年报
            report("2025-06-30", Some(1.22e9), Some(1.22)), // 上年同期累计
        ];
        let v = compute_graham_value(&financials, 118.94, None).expect("TTM 可还原时应可用");
        // TTM EPS = 2.64 + 1.18 − 1.22 = 2.60
        let expected = 2.60 * (8.5 + 2.0 * DEFAULT_GROWTH);
        assert!((v - expected).abs() < 1e-6, "v={v}, expected={expected}");
        // 回归护栏：绝不能退化成「直接用中报累计 EPS」
        let interim_only = 1.18 * (8.5 + 2.0 * DEFAULT_GROWTH);
        assert!(
            (v - interim_only).abs() > 1.0,
            "TTM 未生效，v={v} 仍贴近中报口径值 {interim_only}"
        );
    }

    /// P1-A: 负增长不得被 clamp 抹平为 0（原 clamp(0.0, 0.30) 等于对衰退股默认「零增长」）
    #[test]
    fn graham_does_not_floor_negative_growth_to_zero() {
        let mut financials = vec![report("2025-12-31", Some(5.0e8), Some(1.0))];
        financials[0].revenue_yoy = Some(-50.0); // −50% → g 取下界 −0.30
        let v = compute_graham_value(&financials, 10.0, None).expect("年报口径应可用");
        let expected = 1.0 * (8.5 + 2.0 * -0.30);
        assert!((v - expected).abs() < 1e-6, "v={v}, expected={expected}");
        assert!(v < 1.0 * 8.5, "负增长估值应低于零增长基准 8.5：{v} vs 8.5");
    }

    /// P1-A: bond_yield 配为 0 时不得返回 inf（除零防护）
    #[test]
    fn graham_returns_none_when_bond_yield_non_positive() {
        let financials = vec![report("2025-12-31", Some(5.0e8), Some(1.0))];
        let cfg = ValuationConfig { bond_yield: Some(0.0), ..Default::default() };
        assert!(
            compute_graham_value(&financials, 10.0, Some(&cfg)).is_none(),
            "bond_yield=0 应返回 None 而非 inf"
        );
    }

    /// V74: 全历史 EPS≤0 → None
    #[test]
    fn graham_none_when_no_positive_eps() {
        let financials = vec![
            report("2026-06-30", Some(-8.38e8), Some(-0.5)),
            report("2025-12-31", Some(-3.0e8), Some(-0.18)),
        ];
        assert!(compute_graham_value(&financials, 7.91, None).is_none());
    }

    /// 年报口径过滤: 季报（累计值）不参与均值，避免季度口径污染
    #[test]
    fn annual_filter_excludes_quarterly_reports() {
        let financials = vec![
            report("2026-06-30", Some(-8.38e8), Some(-0.5)), // H1 累计
            report("2026-03-31", Some(-2.0e8), Some(-0.12)), // Q1 累计
            report("2025-12-31", Some(5.0e8), Some(0.30)),
        ];
        let avg = normalized_annual_profit(&financials, 5).expect("应有正年报净利");
        assert!((avg - 5.0e8).abs() < 1e-6, "只统计年报, avg={avg}");
        let eps = normalized_annual_eps(&financials, 5).expect("应有正年报 EPS");
        assert!((eps - 0.30).abs() < 1e-6);
    }

    // ── DCF 模型适用性（2026-09-14）────────────────────────────────────────
    //
    // 这一组测试的共同结构是「**同一份代码、两种数据形态、相反结论**」：
    // 只断言「601166 被判不适用」是不够的 —— 一个无脑返回 `applicable = false`
    // 的实现同样能让那种测试变绿。因此每条正向判据都配一条**反向断言**
    // （低杠杆 + FCF 与净利同向的标的不得被误判），否则「修复」很容易退化成
    // 「所有股票都不适用 DCF」——那是换一种方式让模型撒谎。

    /// 601166 真实形态（2026-09-14 样本 86c7d441）：
    /// 高杠杆 + 净利为正而当期 FCF 为负 + 预测期微幅收缩。
    ///
    /// 对应生产事实：DCF 给 `low/mid/high = 40.17/44.13/49.26`（现价 18.15，
    /// `upsidePct = 143.1`），而 LLM 层给「观望 + 0% 仓位」——同一份输出互相打脸。
    #[test]
    fn dcf_inapplicable_for_leveraged_negative_fcf_shape() {
        let mut latest = report("2026-06-30", Some(411.31e8), Some(1.98));
        latest.debt_ratio = Some(91.6); // 银行结构，不是「财务困境」
        latest.free_cash_flow = Some(-200.0e8); // 净利为正、FCF 为负 ⇒ 符号相反
        latest.revenue_yoy = Some(-0.25);
        let financials = vec![
            latest,
            report("2025-12-31", Some(430.0e8), Some(2.07)),
            report("2024-12-31", Some(420.0e8), Some(2.02)),
            report("2023-12-31", Some(400.0e8), Some(1.93)),
            report("2022-12-31", Some(380.0e8), Some(1.83)),
            report("2021-12-31", Some(350.0e8), Some(1.69)),
        ];
        let (tiers, _, a) = compute_dcf(&financials, shares_of(207.74e8), 18.15, None);

        // ① **零破坏性**：不适用 ≠ 不可用。三档数值仍须产出（面板与旧模板依赖它们），
        //    改判据只应新增标记，不应把估值整块变成 None。
        let (low, mid, high) = tiers.expect("不适用也必须照常产出三档数值（零破坏性）");
        assert!(low <= mid && mid <= high, "档位序不得被新判据破坏");

        // ② 判为不适用，且原因**逐条**可读（不是一句笼统的「模型不适用」）
        let a = a.expect("应回传参数快照");
        assert!(!a.applicable, "高杠杆 + 净利为正而 FCF 为负 ⇒ 必须判不适用");
        let reason = a.inapplicable_reason.as_deref().expect("不适用必须给出原因");
        assert!(reason.contains("资产负债率"), "原因应含杠杆判据: {reason}");
        assert!(reason.contains("符号相反"), "原因应含 FCF 背离判据: {reason}");
        assert_eq!(
            a.applicability_signals.len(),
            2,
            "本次应恰好命中 ①②: {:?}",
            a.applicability_signals
        );

        // ③ 判据 ② 取的是**当期真实** FCF，不是 fallback 代理值 ——
        //    代理值 = 净利均值 × 0.9 ≈ 0.9 × 净利，恒高于 0.3 阈值，会把
        //    「符号相反」这个最强信号整体抹掉。这正是实现里把 `direct_fcf`
        //    提到外层作用域的原因，本断言守住它不被改回去。
        assert!(a.is_fallback_anchor, "当期 FCF ≤ 0 应走 fallback 锚定");

        // ④ 判据 ③ **不**命中：符号一致性约束后永续被压到 0，终值占比 70.6% → ~66%。
        //    把「③ 是否命中」钉死，防止上方常量与字段的文档注释漂移。
        assert!(a.perpetual_clamped_by_negative_growth);
        assert!(
            a.terminal_value_ratio < TERMINAL_RATIO_MAX,
            "约束后终值占比应低于 {}%，实际 {:.1}%",
            TERMINAL_RATIO_MAX * 100.0,
            a.terminal_value_ratio * 100.0
        );
        assert!(
            !a.applicability_signals.iter().any(|s| s.contains("终值现值")),
            "③ 不应命中: {:?}",
            a.applicability_signals
        );
    }

    /// **反向断言**：低杠杆 + FCF 与净利同向的标的**不得**被判不适用。
    ///
    /// 没有这条，实现可以退化成「无条件 `applicable = false`」而全绿。
    #[test]
    fn dcf_applicable_for_low_leverage_healthy_cashflow_shape() {
        let mut latest = report("2025-12-31", Some(700.0e8), Some(55.8));
        latest.debt_ratio = Some(20.0);
        latest.free_cash_flow = Some(900.0e8); // FCF/净利 = 1.29，正常经营区间
        latest.revenue_yoy = Some(12.0);
        let (tiers, _, a) = compute_dcf(&[latest], shares_of(12.56e8), 1500.0, None);
        assert!(tiers.is_some());
        let a = a.expect("应回传参数快照");
        assert!(a.applicable, "低杠杆 + FCF/净利 > 1 不应判不适用: {:?}", a.applicability_signals);
        assert!(a.applicability_signals.is_empty());
        assert_eq!(a.inapplicable_reason, None);
        assert!(!a.perpetual_clamped_by_negative_growth, "正增长不得触发符号约束");
        assert_eq!(a.perpetual_growth, PERPETUAL_GROWTH);
        assert!(!a.is_fallback_anchor);
    }

    /// 判据 ② 的两种形态 + 一条**非**判据，逐档钉死边界。
    #[test]
    fn dcf_fcf_divergence_signal_boundaries() {
        // (a) 量级脱钩：两者皆正但 FCF/净利 = 0.25 < 0.3 ⇒ 命中
        let mut weak = report("2025-12-31", Some(100.0e8), Some(8.0));
        weak.debt_ratio = Some(30.0);
        weak.free_cash_flow = Some(25.0e8);
        let a = compute_dcf(&[weak], shares_of(10.0e8), 50.0, None).2.expect("应回传快照");
        assert!(!a.applicable, "FCF/净利 = 0.25 应判不适用");
        assert!(
            a.applicability_signals.iter().any(|s| s.contains("FCF/净利")),
            "{:?}",
            a.applicability_signals
        );

        // (b) 同形态但越过阈值：0.35 ≥ 0.3 ⇒ **不**命中（阈值不是装饰）
        let mut ok = report("2025-12-31", Some(100.0e8), Some(8.0));
        ok.debt_ratio = Some(30.0);
        ok.free_cash_flow = Some(35.0e8);
        let a = compute_dcf(&[ok], shares_of(10.0e8), 50.0, None).2.expect("应回传快照");
        assert!(a.applicable, "FCF/净利 = 0.35 不应判不适用: {:?}", a.applicability_signals);

        // (c) 净利为正但 FCF **数据缺失** ⇒ 不算命中 —— 缺数据 ≠ 模型不成立。
        //     该情形由 fallback 锚定 + `is_fallback_anchor` 承担置信度衰减，
        //     与「模型前提不成立」是两种不同的降级理由，不可混为一谈。
        let mut missing = report("2025-12-31", Some(100.0e8), Some(8.0));
        missing.debt_ratio = Some(30.0); // free_cash_flow / ocf / capex 全缺
        let a = compute_dcf(&[missing], shares_of(10.0e8), 50.0, None).2.expect("应回传快照");
        assert!(a.applicable, "FCF 缺失不应判不适用: {:?}", a.applicability_signals);
        assert!(a.is_fallback_anchor, "缺失会走 fallback 锚定，由该标记负责降级");
    }

    /// 判据 ③：预测期收缩 + 估值几乎全由永续终值贡献 ⇒ 结论不可检验。
    ///
    /// 越过 70% 需要**低折现率** —— 这不是造数据，而是低利率环境下的真实形态
    /// （`dcf_discount_rate` 是面板可配的扁平参数，不是写死的常量）。
    /// 同一份财报只改折现率 6.0% → 8.5%，判据从命中变不命中 ⇒ 证明阈值真有区分度。
    #[test]
    fn dcf_terminal_dominance_signal_tracks_discount_rate() {
        let build = || {
            let mut r = report("2025-12-31", Some(10.0e8), Some(0.6));
            r.debt_ratio = Some(30.0);
            r.free_cash_flow = Some(6.0e8); // FCF/净利 = 0.6 ⇒ 不触发判据 ②
            r.revenue_yoy = Some(-6.05); // 预测期收缩
            vec![r]
        };
        // 低折现率（6%）⇒ 终值占比 > 70% ⇒ 命中 ③
        let cfg_low =
            ValuationConfig::from_flat_arguments(&serde_json::json!({ "dcf_discount_rate": 6.0 }))
                .expect("扁平参数应可解析");
        let a =
            compute_dcf(&build(), shares_of(10.0e8), 15.0, Some(&cfg_low)).2.expect("应回传快照");
        assert!(
            a.applicability_signals.iter().any(|s| s.contains("终值现值")),
            "折现率 6% 下终值占比 {:.1}% 应命中 ③: {:?}",
            a.terminal_value_ratio * 100.0,
            a.applicability_signals
        );
        assert!(!a.applicable);

        // 默认折现率（8.5%）⇒ 占比降到 ~63% ⇒ **不**命中（反向断言）
        let a = compute_dcf(&build(), shares_of(10.0e8), 15.0, None).2.expect("应回传快照");
        assert!(a.applicable, "8.5% 折现率下不应命中 ③: {:?}", a.applicability_signals);
        assert!(a.terminal_value_ratio < TERMINAL_RATIO_MAX);
    }

    /// 永续增长率符号一致性：**同一份财报、同一个配置**，仅因增长方向不同而分道。
    #[test]
    fn perpetual_growth_sign_consistency_forces_zero_on_shrink() {
        let mk = |yoy: f64| {
            let mut r = report("2025-12-31", Some(10.0e8), Some(0.6));
            r.debt_ratio = Some(30.0);
            r.free_cash_flow = Some(6.0e8);
            r.revenue_yoy = Some(yoy);
            vec![r]
        };
        // 收缩 ⇒ 永续被压到 0，配置原值保留在对账字段
        let (shrink, _, a) = compute_dcf(&mk(-6.05), shares_of(10.0e8), 15.0, None);
        let a = a.expect("应回传快照");
        assert_eq!(a.growth, -0.0605);
        assert_eq!(a.configured_perpetual_growth, PERPETUAL_GROWTH);
        assert_eq!(a.perpetual_growth, 0.0);
        assert!(a.perpetual_clamped_by_negative_growth);

        // 增长 ⇒ 配置值原样生效，不受约束（反向断言）
        let (_, _, b) = compute_dcf(&mk(10.8), shares_of(10.0e8), 15.0, None);
        let b = b.expect("应回传快照");
        assert_eq!(b.perpetual_growth, PERPETUAL_GROWTH);
        assert!(!b.perpetual_clamped_by_negative_growth);

        // 约束必须**真实改变数值**，不能只翻一个标记 ——
        // 用同一公式把永续塞回 +4% 复算，若两侧接近则说明 `perpetual_growth` 根本没被用上。
        let fcf_ps = 6.0e8 / 10.0e8;
        let (g, p, d, n) = (-0.0605_f64, PERPETUAL_GROWTH, DISCOUNT_RATE, FORECAST_YEARS);
        let mut pv = 0.0;
        let mut cf = fcf_ps;
        for y in 1..=n {
            cf *= 1.0 + g;
            pv += cf / (1.0 + d).powi(y);
        }
        let with_uncapped_perpetual = pv + (cf * (1.0 + p) / (d - p)) / (1.0 + d).powi(n);
        let mid_clamped = shrink.expect("应可用").1;
        assert!(
            with_uncapped_perpetual > mid_clamped + 1.0,
            "压回前 mid({with_uncapped_perpetual}) 与压回后({mid_clamped}) 无显著差异 \
             ⇒ `perpetual_growth` 未参与实际计算，约束是空转"
        );
    }
}
