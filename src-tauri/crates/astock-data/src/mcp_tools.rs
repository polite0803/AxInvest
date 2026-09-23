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
                    "kline_json": { "type": "string", "description": "上游K线节点输出的JSON" },
                    "period": { "type": "string", "description": "K线周期：daily/weekly/monthly，决定技术指标与评分所在周期（PROPOSAL 阶段2 四周期独立决策）", "default": "daily" }
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
            "name": "compute_valuation_band",
            "description": "估值分位带：按东财历史估值日序列算 PE/PB/PS 的 5/10/25/50/75/90/95 分位与当前分位，返回 verdict 与 metricPe.currentPercentile。工作流用它当 DCF 之外的**独立估值锚腿**（历史分位高 ⇒ 折价、低 ⇒ 溢价）",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "stock_code": { "type": "string", "description": "6位股票代码" },
                    "years": { "type": "number", "description": "回溯窗口(年)，默认 5" }
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
            // 允许调用方传入 kline_json（避免重复拉取）；若未提供则现场拉取 K 线。
            // PROPOSAL 阶段 2（四周期独立决策）：`period` 决定技术指标与评分所在周期
            //（daily/weekly/monthly）。缺省 daily —— 与历史行为一致，无参路径零回归。
            let period = arguments["period"].as_str().unwrap_or("daily");
            let period = if period == "weekly" || period == "monthly" {
                period
            } else {
                "daily"
            };
            let klines = if let Some(kj) = arguments["kline_json"].as_str() {
                serde_json::from_str::<Vec<crate::types::KLine>>(kj)
                    .map_err(|e| format!("kline_json 解析失败: {e}"))?
            } else {
                client.get_klines(code, period, 120).await.map_err(|e| e.to_string())?
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
                // 2026-09-21 拆字段：原 `fundamentalAdjustment` **一名双源**
                // （基本面 PE/PB/ROE 与行业相对偏离都往它累加）⇒ 字段名给了 LLM 一个
                // 错的归因。现补两个键：行业分量 + 合计，满足恒等式
                // `totalAdjustment == fundamentalAdjustment + industryAdjustment`。
                // ⚠ `fundamentalAdjustment` 的**语义已修正**（由「合计」变为「仅基本面」）；
                //   要旧口径读 `totalAdjustment`。`total` 字段数值不受影响。
                "industryAdjustment": score_json["industryAdjustment"],
                "totalAdjustment": score_json["totalAdjustment"],
                "signal": score_json["signal"],
                "signalCode": score_json["signalCode"],
                // ── #7 新增: 别名 + 原始指标 + 占位字段 ──
                "totalScore": score_json["total"], // 别名,供 input_mapping 引用
                "currentPrice": latest_price,       // 最新收盘价
                "period": period,                   // 本次评分所在周期（daily/weekly/monthly），供多周期决策区分档位
                "indicators": ind_json,             // 完整技术指标(ma5/ma20/bias_ma5/macd_dif/rsi14/boll_upper 等)
                // kline_json: K 线原始数据（数量=limit），供 trader 节点的 ATR/Kelly/MC 工具使用
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
            // 2026-09-21: 返回值追加「实际生效假设」，供下游判断该腿是否可信
            //   （`growth_clamped_upper` ⇒ 增长率顶死上界 ⇒ 内在价值系统性偏低）。
            let (graham_value, graham_assumptions) =
                match compute_graham_value(&financials, current_price, valuation_config.as_ref()) {
                    Some((v, a)) => (Some(v), Some(a)),
                    None => (None, None),
                };

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
            // 假设快照与可用性同生命周期：量程判定不可用时一并置 null，
            // 避免下游拿到「其实没生效」的假设（`assumptions != null ⟺ 该腿可用`）。
            let graham_assumptions = if graham_usable {
                graham_assumptions
            } else {
                None
            };

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
                value_signal_of(mos_pct, f_score, moat_score, oe_yield).to_string()
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
                // ── 2026-09-23：`upsidePct` 基准由**中性档 `mid`** 改为**保守档 `low`** ──
                //
                // 病根（用户实证 300642 透景生命）：三段 DCF 自报估值域
                //   `low/mid/high = 24.76 / 40.53 / 71.89`（现价 21.02）—— **宽 2.90 倍**。
                //   该宽度来自**人为乘子**（预测期 `g × 0.6 / × 1.5`、永续 `p × 0.7 / × 1.3`）
                //   经终值项 `1/(d − p)` 放大 —— 300642 终值占 **84.4%**，即估值几乎全由
                //   「永续增长率」这一个不可验证的假设决定。
                //   而决策链**只取 `mid`** 算 `upsidePct = +92.8%`，等于把「三个任意假设中
                //   的中间那个」当成点估计 ⇒ **模型自报的 ±2.9 倍不确定度对决策完全透明**。
                //   同一份 payload 里的 `low`（24.76，仅比现价高 17.8%）与 `high`（71.89）
                //   此前**从未参与任何因子计算**。
                //
                // 为什么改**基准**而不是加「宽度衰减系数」：
                //   加系数需要引入一个**新的标定常数**（「宽度 → 折扣率」的映射），而该映射
                //   本身没有客观依据 —— 那与本次要修的病**同源**（用任意乘子表达不确定性）。
                //   改基准则**不引入任何新常数**：它只是把已有的 `low` 从「展示用」提升为
                //   「决策用」，且语义正确 —— **安全边际必须在最保守假设下成立**，
                //   而不是在中性假设下成立。`low` 正是本模型自报的保守锚。
                //
                // ⚠️ 兼容性：`dcf_low/mid/high` 由同一个 `dcf_usable` 分支产出（见上方遮蔽），
                //   **三者同生同灭** ⇒ 换基准不会产生「`low` 缺失而 `mid` 存在」的新形态。
                // ⚠️ 本键**语义随之收紧**：由「中性档上行空间」变为「**保守档**上行空间」。
                //   需要中性档口径的消费方（LLM 点估计）请读新增的 `midUpsidePct`
                //   —— 信息未丢失，只是不再冒充决策输入。
                "upsidePct": match dcf_low {
                    Some(low) if current_price > 0.0 => {
                        json!(round1((low - current_price) / current_price * 100.0))
                    },
                    _ => serde_json::Value::Null,
                },
                // 中性档上行空间（2026-09-23 新增）：**仅供展示 / LLM 点估计参考**，
                //   **不参与** f5 因子 —— 它正是 `upsidePct` 改动前的旧口径，保留以避免信息丢失
                //   （也便于审计「改基准前后差多少」）。
                "midUpsidePct": match dcf_mid {
                    Some(mid) if current_price > 0.0 => {
                        json!(round1((mid - current_price) / current_price * 100.0))
                    },
                    _ => serde_json::Value::Null,
                },
                },
                "graham": {
                    "intrinsicValue": num_or_null(graham_value, round2),
                    "upsidePct": match graham_value {
                        Some(g) if current_price > 0.0 => json!(round1((g - current_price) / current_price * 100.0)),
                        _ => serde_json::Value::Null,
                    },
                    // 2026-09-21：实际生效假设快照（与 `dcf.assumptions` 同源动机）。
                    // ⚠️ 消费者**不要**按这里的文案/数值自行重算结论——
                    //   `growthClampedUpper` 是给下游做「该腿降信」的布尔信号，
                    //   不是让 Rhai 重新推导估值。
                    "assumptions": match graham_assumptions {
                        Some(a) => json!({
                            "growth": a.growth,
                            "growthClampedUpper": a.growth_clamped_upper,
                            "growthClampedLower": a.growth_clamped_lower,
                            "growthFromDefault": a.growth_from_default,
                            // 百分数（公式 `4.4 / bondYield` 的修正项分母）
                            "bondYield": a.bond_yield,
                            // ⚠️ 口径提醒：上面的 `growth` 是**小数**（0.12），而本公式的
                            //   `g` 与 `bondYield` 都是**百分数** —— 自证/复算时必须换算
                            //   （乘数 = `8.5 + 2×12 = 32.5`，**不是** `8.5 + 2×0.12`）。
                            //   2026-09-22 量纲修复前生产端就栽在这个混淆上。
                            //   为免下游再算错，直接给出 `multiplier`：
                            //   消费端（含 LLM）**不要**自己从 `growth` 推乘数。
                            "formula": "EPS × (8.5 + 2×g%) × 4.4 / bondYield，其中 g% = growth × 100",
                            "multiplier": round1(8.5 + 2.0 * a.growth * 100.0),
                        }),
                        None => serde_json::Value::Null,
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
        // ── 估值分位带（R3-C）──
        //
        // ⚠ **同名不同层**（读到此名的人最容易踩的坑）：
        //   · `commands::stock_analysis::compute_valuation_band` —— **Tauri 命令**，
        //     走 `State<AppState>`，读本机 `financial_snapshots` 表（样本不足/陈旧时**先回填**），
        //     前端 `ValuationBandChart` 与设置面板走它；**它不能被工作流节点调用**
        //     （`#[agent_command]` 只登记元数据，不存在「命令 → 工具」的桥）。
        //   · 本分支 —— **MCP 工具**，走 `ToolRegistry`，在线取数（`get_valuation_history`，
        //     自带 12h 缓存）且**不落库**，工作流 `tool_node("t-valuation-band")` 走它。
        //
        // 两条路径的**窗口口径**刻意共享（`since_date_from_years` + `clip_valuation_history`），
        // 且底层是同一次 `AStockClient::get_valuation_history`（同一份缓存）⇒ 同一交易日下
        // 两者的分位一致；差异只在「是否把序列落库」。
        // 背景：V79 首次实现时该节点直接写了 `compute_valuation_band` 当工具名，
        // 但工具表里没有它 ⇒ `ToolResolver` 返 `None` ⇒ `core.rs` 的 Failed 分支
        // `emit degraded: true` **静默吞掉**（节点 completed、输出为空、无报错）
        // ⇒ `valuation_pe_percentile` 恒空 ⇒ band 腿成了**永久空壳**。
        "compute_valuation_band" => {
            let code = parse_code(arguments);
            let code = code.as_str();
            if code.is_empty() {
                return Err("compute_valuation_band 缺少 stock_code 参数".to_string());
            }
            // 与 `parse_code` 同款容错：LLM/模板可能把 years 传成字符串
            let years = match &arguments["years"] {
                serde_json::Value::Number(n) => n.as_u64().map(|v| v as u32),
                serde_json::Value::String(s) => s.trim().parse::<u32>().ok(),
                _ => None,
            }
            .unwrap_or(5);
            let snaps =
                client.get_valuation_history(code, years).await.map_err(|e| e.to_string())?;
            // ⚠ 必须裁剪：`get_valuation_history(years)` 会多取约 2 页（≈2 年），
            //   不裁的话窗口比声明值宽近一倍 ⇒ 分位与命令层不一致。
            let since_date = crate::valuation_band::since_date_from_years(years);
            // `clip_valuation_history` 会把结果**归一到升序**（vendor 原始顺序是降序，
            // 命令层读表是升序）—— 所以下面 `snaps.last()` 才是"窗口内最新一条"。
            // ⚠ 不要在这条链上换掉裁剪函数或自己 filter，否则 `current` 会取到**最旧**那天。
            let snaps = crate::valuation_band::clip_valuation_history(snaps, &since_date);
            let band = crate::valuation_band::compute_valuation_band(code, &snaps, snaps.last());
            serde_json::to_string(&band).map_err(|e| e.to_string())
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
    // 2026-09-21 修复：原实现只在**首档**写 `pe_val > 0.0`，后两档裸比 `< 25.0` /
    //   `< 50.0` —— 负 PE（亏损企业）会命中第二档白拿 +10 分「估值合理性」。
    //   该缺陷此前被 vendor 的 `filter(|v| *v > 0.0)` 掩盖（pe 恒为 None ⇒ 整块不进），
    //   放开负 PE 后必须显式守卫，否则「亏损」反而成为估值加分项。
    if let Some(pe_val) = pe {
        if pe_val > 0.0 {
            if pe_val < 15.0 {
                score += 15;
            } else if pe_val < 25.0 {
                score += 10;
            } else if pe_val < 50.0 {
                score += 5;
            }
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
/// 估值参数说明。
///
/// ## 取值依据（2026-09-23 补 —— 此前本节只论证「四处同步」，**不论证「为什么是这个数」**）
///
/// 折现率拆成两个**具名**分量，并由**编译期断言**把三者锁成等式关系
/// （`DISCOUNT_RATE == RISK_FREE_RATE + EQUITY_RISK_PREMIUM`，见下方断言）：
///
/// | 常量 | 值 | 依据 |
/// |---|---|---|
/// | `RISK_FREE_RATE` | 2.5% | A 股 10 年期国债收益率中枢，即本模型的**无风险基准** |
/// | `EQUITY_RISK_PREMIUM` | 6.0% | A 股股权风险溢价常用区间 5%–6%，取**上沿**（偏保守一侧） |
/// | `DISCOUNT_RATE` | **8.5%** | = 上方两者之和（**编译期断言锁死**，非独立取值） |
/// | `PERPETUAL_GROWTH` | 2.0% | ≈ 长期通胀中枢（低于 PBoC 目标上沿 3%） |
/// | `DEFAULT_GROWTH` | 12% | 营收增速缺失时的兜底；A 股优质公司中期增速的中位量级 |
/// | `MIN_GROWTH`/`MAX_GROWTH` | −30% / +30% | 只作**异常值围栏**，不是预测 |
///
/// ## ⚠️ 永续增长率的硬约束 `p ≤ RISK_FREE_RATE`（2026-09-23 订正）
///
/// 原值 `PERPETUAL_GROWTH = 4%` **违反本模型自身申报的无风险利率 2.5%**：它隐含
/// 「该企业永续增速高于无风险利率」⇒ 高于经济体长期名义增速 ⇒ 等价于假设该企业
/// 在**整个永续期内不断吞掉经济体份额**（终值 → ∞）。
/// Damodaran 的终值约束即 `g_terminal ≤ risk-free rate`。
///
/// 定量后果（300642，现价 21.02）：`p = 4%` → `mid = 40.53`（`upsidePct = +92.8%`）；
/// `p = 2%` → `mid ≈ 31.4`（**−22.6%**）。即修正前「低估 92.8%」这一结论里，
/// 约 **22 个百分点**纯粹来自一个无依据、且与自身前提冲突的参数取值。
///
/// ## 反解验证（2026-09-23）
///
/// 由 `high/mid` 与 `low/mid` 两个比值联立，反解出 300642 的 `growth = 17.78%`、
/// 隐含 `FCF/股 = 0.9819` 元；代入本节公式复算得 `24.76 / 40.53 / 71.89`，
/// 与面板显示**逐位吻合**（`low/mid` 偏差 0.01%）⇒ 本节公式就是生产公式，
/// 下文各处的敏感度数字是复算真值，不是估算。
///
/// ## ⚠️ 本节常量是**估值缺省参数的唯一真相源**（`pub const`，2026-09-22）
///
/// 这些值此前在多处被**手抄**，且互不一致 —— 校准（2026-09-12：折现率 10→8.5、
/// 永续 3→4、默认增长 8→12、下界 +2→−30）只落到其中一部分，另一些停在旧值上，
/// 造成「同一个参数在不同链上取不同值」：
///
/// | 消费端 | 位置 | 机制 |
/// |---|---|---|
/// | 模块兜底（本文件） | `ValuationConfig::{perpetual_growth,discount_rate,…}` | `unwrap_or(<本常量>)` |
/// | 设置页估值参数（主 crate） | `commands/stock_analysis.rs::ValuationParams::default()` | **派生自本常量**（勿再手抄） |
/// | 模板变量默认值 | `stock_analysis_setup/seed_variables.rs::DEFAULT_DCF_*_PCT` | **派生自本常量 ×100** |
/// | 决策层估值配置 | `analysis-engine::decision::ValueConfig::{default_dcf_*,Default}` | **派生自本常量 ×100**（2026-09-23 收敛，此前是**第五份手抄**） |
/// | 前端兜底 | `components/settings/StockAnalysisConfigPanel.tsx::DEFAULT_VALUATION_PARAMS` | 前端无法引用 Rust ⇒ 手抄，由等式门禁守卫 |
///
/// **改动纪律**：改本常量即改「全部标的的估值口径」，必须同步核验上表四处。
/// 主 crate 侧有 `valuation_defaults_are_single_sourced` 等式测试兜住前两处。
///
/// **下界由 `+2%` 改为 `-30%`（2026-09-12，PLAN P0-F 方案 A）**。原值 `+0.02` 是**正数**，
/// 而 `growth = revenue_yoy.clamp(min_growth, max_growth)` ⇒ **营收负增长的标的被抬成
/// 「确定性 +2% 增长」**，且**越差的公司偏置越大**。DB 实证（603353）：`revenue_yoy = −6.05%`
/// 被抬到 `+2%`，`dcf.mid` 因之偏高约 42%（修复后 5.15 → 3.61）。
/// 同源的 `compute_graham_value` 已于 P1-A 把下界由 `0.0` 改为 `-0.30`（其注释明写
/// 「把负增长抹平为 0……反而**高估**其内在价值」）—— 本次是补齐 DCF 侧的**漏修**。
/// 无风险利率（小数口径）。
///
/// ## 取值与来源（2026-09-23 订正 —— 旧值取错了利率品种）
///
/// | 项 | 值 | 来源 |
/// |---|---|---|
/// | **本文取值** | **1.7%** | 中债 **10 年期**国债到期收益率，2026-09-22 实测 **1.675%**（chinabond 日评 / 新华财经 / 东财多源一致），取整 1.7% |
/// | 旧值（错） | 2.5% | 实为 **1 年期 MLF 政策利率 2.50%**（8/25 操作） |
///
/// **为什么旧值是错的（可判定，非观点）**：DCF 的无风险利率必须与**被折现现金流的久期**
/// 匹配。本模型 = 预测期 5 年 + 永续终值 ⇒ 现金流久期在 **10 年以上** ⇒ 对应 **10 年期国债**。
/// 而 2.5% 是 **MLF（1 年期政策利率）** —— 政策利率是央行主动操作的利率，不是市场无风险
/// 利率。两者当期相差 **82.5bp**（2.50% vs 1.675%），是数量级错误而非精度问题。
///
/// ⚠️ **旧注释声称的口径与取值不符**：上一行原写「A 股 10 年期国债收益率中枢」，而取的是
/// MLF —— 这正是「依据不可判定」的形态（`MEMORY-RULES.md` #687）：读的人会以为 10Y 就是
/// 2.5%，无从反驳。
///
/// ⚠️ **本值时变**，不得沿用不核对：每次修订须重新核对中债 10Y
/// （`yield.chinabond.com.cn`，或中债日评），并在上表标注**核对日期**。
/// 下一行的编译期断言会强制它与 `EQUITY_RISK_PREMIUM` / `DISCOUNT_RATE` 三者一致。
pub const RISK_FREE_RATE: f64 = 0.017;
/// 股权风险溢价（小数口径）—— A 股常用区间 5%–6%，取上沿即偏保守一侧。
pub const EQUITY_RISK_PREMIUM: f64 = 0.06;
/// 永续增长率 —— **由本模型自身的两条硬约束反推**，不是独立取的「通胀中枢」估计。
///
/// 两条约束（都是可判定的，不是偏好）：
/// 1. `p ≤ RISK_FREE_RATE`（Damodaran 稳定期约束 `g_terminal ≤ r_f`）—— 一个经济体的
///    永续增速不可能高于其无风险利率；
/// 2. `p × HIGH_PERPETUAL_SCALE_POS ≤ RISK_FREE_RATE` —— 乐观档取 `p × 1.3`，若该值越过
///    `MAX_PERPETUAL_GROWTH`（= `r_f`），乐观档的永续增长率会被**静默砍回**，而面板文案
///    仍声称「×1.3」⇒ **口径与实算不符**。
///
///   ⚠️ 该缺陷 2026-09-23 **先被在增长率维度修掉、又在永续维度复活**：
///   增长率维度当时是 `high_growth` 上界取 `MAX_GROWTH` 而非 `MAX_GROWTH × 1.5` ⇒
///   `g = 30%` 时实际乘子 1.0 而文案写 ×1.5（已改为 `max_growth × 1.5`）；
///   本轮把 `p` 取 `1.5%` 且 `MAX_PERPETUAL_GROWTH` 锚到 `r_f = 1.7%` 后，
///   `1.5% × 1.3 = 1.95% > 1.7%` ⇒ 同型缺陷**在另一维复发**（实际乘子 1.133）。
///
/// 取上界：由约束 2 解出 `p ≤ r_f / 1.3 = 1.3077%` ⇒ 取 **0.1pp 整 = `1.3%`**
/// （`1.3% × 1.3 = 1.69% ≤ 1.7%` ✅ 不再被砍）。下一行的编译期断言锁死该不变量
/// ⇒ `r_f` 或乘子任一变动都会**在编译期报错**，强制重新反推 `p`，不会静默失效。
///
/// ⚠️ **本值随 `RISK_FREE_RATE` 联动，禁止独立调整**（`r_f` 是时变的，见上一节）。
///
/// ## 弹性口径订正（2026-09-23，**推翻本文件上一版的两处数字**）
///
/// 上一版此处写「`E_d = −1.96`、`E_p = 0.78`」并据此论证「`p` 降 0.5pp ⇒ 估值 ↓」。
/// 那两个数是在**旧的 `p = 4%`** 上测的，且 `E_p` **不是常数**：
/// 由 `TV = FCF₅(1+p)/(d−p)` 可解析得 `E_p ∝ p / (d−p)²` —— `p` 越小、`d−p` 越大，
/// `E_p` 塌得越快。实测（300642，`g = 17.78%`；复算脚本 `output/sci21-structural-at-final-params.mjs`）：
///
/// | `p` / `d` | `E_g` | `E_p` | `E_d` | 排序 |
/// |---|---|---|---|---|
/// | 4.0% / 8.5%（最早） | 0.711 | 0.783 | −1.963 | `d > p > g` |
/// | 2.0% / 8.5% | 0.695 | **0.257** | −1.389 | `d > g > p` |
/// | 1.5% / 7.7% | 0.698 | **0.204** | −1.318 | `d > g > p` |
/// | **1.3% / 7.7%（终态）** | 0.697 | **0.171** | −1.280 | `d > g > p` |
///
/// ⇒ **唯一稳健的排序结论是「折现率的弹性始终最大」**（`|E_d|/|E_g|` 从 2.76 降到 1.84，
/// 即约 **1.8–2.0 倍**，始终为最大值）；「`p` 的弹性排第二」会随 `p` 取值翻转
/// （`p` 从 4% 降到 2% 时 `E_p` 由 0.783 **塌到 0.257**），**不得作为依据引用**。
/// 另需区分**弹性**（局部）与**水平效应**（`p` 在终值分母里，4%→1.3% 使 `mid` 降约 24%）——
/// 弹性小 ≠ 影响小，两者不可互推。
pub const PERPETUAL_GROWTH: f64 = 0.013;
/// 折现率 = 无风险利率 + 股权风险溢价 = **7.7%**（= 1.7% + 6.0%）。
///
/// ⚠️ 刻意写成**字面量 + 编译期等式断言**（下一行），而不是 `RISK_FREE_RATE +
/// EQUITY_RISK_PREMIUM` 表达式：IEEE754 下浮点求和与字面量可能差一个 ULP
/// （上一版 `0.025 + 0.06 = 0.08499999999999999` ≠ `0.085`）⇒ 会让所有按字面量比对的
/// 等式门禁（`check-valuation-defaults-parity.mjs` 用 `===`）与已落库快照出现无意义偏差，
/// 而精度上毫无收益。断言在**编译期**锁死等式关系 ⇒ 两个分量与总和三者**无法各自漂移**，
/// 这比写成表达式或加运行时测试都更强。
/// 注：当前取值下 `0.017 + 0.06` 恰等于 `0.077`（差 0），断言仍保留作为防漂移闸。
pub const DISCOUNT_RATE: f64 = 0.077;
/// 编译期锁：`DISCOUNT_RATE` 必须恰为两分量之和（1e-12 容差吸收浮点求和误差）。
/// 手写绝对值而非 `f64::abs()` —— 避免依赖 const-float-method 的稳定版本。
const _: () = assert!(
    {
        let diff = DISCOUNT_RATE - (RISK_FREE_RATE + EQUITY_RISK_PREMIUM);
        let abs = if diff < 0.0 { -diff } else { diff };
        abs < 1e-12
    },
    "DISCOUNT_RATE 必须 = RISK_FREE_RATE + EQUITY_RISK_PREMIUM（拆分后三者不得各自漂移）"
);
pub const DEFAULT_GROWTH: f64 = 0.12;
pub const MIN_GROWTH: f64 = -0.30;
pub const MAX_GROWTH: f64 = 0.30;
/// 格雷厄姆公式中 AAA 企业债收益率的缺省基准（**百分数**口径，公式 `4.4 / bond_yield` 的分母）。
///
/// 与上面 5 个小数量纲的常量不同源，故单独列出；此前该值在 [`ValuationConfig::bond_yield`]
/// 里以裸字面量 `4.4` 出现，主 crate 的 `ValuationParams::default()` 又手抄一份。
pub const DEFAULT_BOND_YIELD: f64 = 4.4;

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

/// 三档对**永续增长率**的方向缩放因子（2026-09-23 提为具名常量）。
///
/// 原文以裸字面量 `0.7` / `1.3` 内联在 `low_perpetual` / `high_perpetual` 两行里。
/// 提名的唯一理由是**让不变量可断言** —— 见下方 `const _` 断言：
/// `PERPETUAL_GROWTH × HIGH_PERPETUAL_SCALE_POS ≤ RISK_FREE_RATE`。
/// 裸字面量时这条不变量既无法表达、也无法在编译期锁住，直接导致乐观档被静默封顶
/// （详细复盘见 [`PERPETUAL_GROWTH`] 的文档）。
const LOW_PERPETUAL_SCALE_POS: f64 = 0.7;
const HIGH_PERPETUAL_SCALE_POS: f64 = 1.3;

/// 编译期锁：乐观档永续增长率**不得被 `MAX_PERPETUAL_GROWTH` 砍掉**。
///
/// 判据：`p × 1.3 ≤ MAX_PERPETUAL_GROWTH`。若违反，`high_perpetual` 会被 `.min()` 截断，
/// 而面板文案仍声称「永续增长率 ×1.3」⇒ **口径与实算不符**（同族缺陷在增长率维度的复盘见
/// `max_growth_high` 处注释）。
///
/// ⚠️ 为什么必须落在这里：`.min()` 是**静默**的，改动 `RISK_FREE_RATE` 或乘子都不会报错，
/// 只会让乐观档悄悄变窄 —— 这正是本项目「修了几次都没用」的高发形态
/// （`MEMORY-RULES.md` #687：依据必须可判定且配闸）。放在编译期 ⇒ 任何一侧漂移即编译失败。
const _: () = assert!(
    {
        let hs = PERPETUAL_GROWTH * HIGH_PERPETUAL_SCALE_POS;
        let ls = PERPETUAL_GROWTH * LOW_PERPETUAL_SCALE_POS;
        let room = MAX_PERPETUAL_GROWTH - hs;
        // 同时要求乐观档严格大于基准档（否则该档与基准重合，三档退化成两档）
        (hs <= MAX_PERPETUAL_GROWTH) && (ls < PERPETUAL_GROWTH) && (room >= 0.0)
    },
    "永续增长率不变量被破坏：须同时满足 `p × HIGH_PERPETUAL_SCALE_POS ≤ MAX_PERPETUAL_GROWTH` \
     与 `p × LOW_PERPETUAL_SCALE_POS < p`。违反则乐观档被静默封顶、或三档退化成两档。\
     处置：按 `p ≤ MAX_PERPETUAL_GROWTH / HIGH_PERPETUAL_SCALE_POS` 重新反推 PERPETUAL_GROWTH。"
);

/// 保守档永续增长率下限：**不允许负永续增长**。
///
/// 原实现用 `min_growth / 2.0` 作下限，在 `MIN_GROWTH` 转负后会变成 `-0.15`
/// —— 等于允许「公司永久萎缩」的终值假设，DCF 终值项失去经济含义。
/// 故提为独立常量并固定在 0（`perpetual_growth` 本身经 `pct()` 守卫恒为正，此下限只是兜底）。
const MIN_PERPETUAL_GROWTH: f64 = 0.0;
/// 永续增长率的**硬上限**：`p ≤ RISK_FREE_RATE`（Damodaran 稳定期约束）。
///
/// **2026-09-23 改为锚定 [`RISK_FREE_RATE`]**（原为裸 `0.05`）。原取值的由来是
/// 「刚好压住 `PERPETUAL_GROWTH × 1.3 = 0.04 × 1.3 = 0.052`」—— 它只对
/// `PERPETUAL_GROWTH = 4%` 这一个具体取值成立，于是形成**脆弱耦合**：
/// `PERPETUAL_GROWTH` 一改，被截断的样本集会**静默变化**，而上限本身没有独立含义。
///
/// 锚定后上限的含义与经济含义一致（**永续增速不得超过无风险利率**）。但**锚定本身不够** ——
/// 2026-09-23 的下一轮把 `p` 取 `1.5%` 后 `1.5% × 1.3 = 1.95% > r_f = 1.7%`，
/// 乐观档又被静默截断（实际乘子 1.133 而文案写 ×1.3）。⇒ 真正的护栏是
/// 「`p` 由 `r_f / HIGH_PERPETUAL_SCALE_POS` 反推」+ 模块级编译期断言，
/// 见 [`PERPETUAL_GROWTH`] 文档与本节下方的 `const _` 断言。
///
/// ## 三处消费者的完整清单（改本常量前必读）
///
/// | # | 位置 | 语义 |
/// |---|---|---|
/// | 1 | `compute_dcf`：`perpetual_growth.min(MAX_PERPETUAL_GROWTH)` | **基准档**也受约束 |
/// | 2 | `compute_dcf`：`high_perpetual` 的 `.min(...)` | 乐观档 |
/// | 3 | `applicability_signals`（判据⑤） | 配置值越界时**上报并退出该腿** |
///
/// ⚠️ 第 1 处是 2026-09-23 补的：此前本常量**只作用于乐观档**，基准档的
/// `perpetual_growth` 只受利差钳位管 ⇒ 用户配 `dcf_perpetual_rate = 5%` 时
/// `mid` 直接用 5%（越过 `r_f`），**静默违反模型自己的前提**。
///
/// 不变量：必须 < `DISCOUNT_RATE`，否则终值分母落到地板、估值失真
/// （`0.017 < 0.077` ✓）。⚠️ 这只是**常量之间**的不变量，管不住用户配置通道
/// （`dcf_perpetual_rate` 经 `pct()` 只守 `0 < raw ≤ 100`，可配出 `p ≥ d`）——
/// 该缺口由 [`MIN_TERMINAL_SPREAD`] 在 `compute_dcf` 内兜住，且**上报**而非静默。
const MAX_PERPETUAL_GROWTH: f64 = RISK_FREE_RATE;

/// 终值分母 `d − p` 的**利差地板**。
///
/// 为什么必须有：终值 `= FCF₅ × (1+p) / (d − p)`，分母是**差值** ⇒ `p → d` 时
/// 终值 → ∞。而 `p` 与 `d` 都来自**用户可配**的扁平参数
/// （`dcf_perpetual_rate` / `dcf_discount_rate`），`pct()` 只守 `0 < raw ≤ 100`
/// ⇒ `p ≥ d` 是**完全可达**的配置。
///
/// 原地板是裸字面量 `0.001`：此时 `p = 100%`（`pct()` 允许）× `d = 8.5%`
/// 会把终值放大到 `FCF₅ × 2000`（正常利差 6.5pp 下约 15 倍，即 **130 倍**），
/// 且**静默无日志** —— 与 002837 那条 `upsidePct = +618,509,774.8%` 的观测形态同级
/// （那条的真因是 `total_shares` 量纲错、已另行修复，但**同一量级的放大器**在这里仍然开着）。
///
/// 取 1.5pp：对应终值倍数上限 `1 / 0.015 ≈ 67×`，已是「估值由差值独裁」的量级。
/// 正常配置下（`p = 1.3%`、`d = 7.7%`）利差 6.4pp，本地板**不参与**。
const MIN_TERMINAL_SPREAD: f64 = 0.015;

/// 悲观情景的**折现率上浮**（「要求回报更高」）。
///
/// 依据：三档的语义是**情景**而非单参数敏感性 —— 悲观情景下经营假设（`g`、`p`）
/// 与要求回报同时不利才是自洽的。而折现率的弹性是三者中最大的：
///
/// | 参数 | 300642 弹性（估值变动 % / 参数变动 %） |
/// |---|---|
/// | 折现率 `d` | **−1.28** |
/// | 预测期增长率 `g` | 0.70 |
/// | 永续增长率 `p` | 0.17 |
///
/// ⚠️ **上表是终态参数（`p = 1.3%`、`d = 7.7%`）的实测值**，复算脚本
/// `output/sci21-structural-at-final-params.mjs`。此前本节引的是 `p = 4%` 时测的
/// `−1.96 / 0.78 / 0.71` —— `E_p` **不是常数**（`E_p ∝ p/(d−p)²`），
/// `p` 从 4% 降到 1.3% 时它由 0.783 **塌到 0.171** ⇒ 旧数字不可直接引用。
/// 唯一稳健的结论是**折现率弹性始终最大**（`|E_d|/|E_g|` = 1.84–2.00）。
///
/// 定量（终态参数）：`d` 仅上浮 1pp（相对变动 +13%）就使估值从 36.85 变到 26.36
/// （**1.40 倍**），**大于** `p` 整个 ×0.7~×1.3 档的 **1.11 倍**。把它排除在区间之外，
/// 等于宣称「区间只覆盖弹性第二和第三的参数」⇒ 区间宽度**归因错误**：面板把宽度归因于
/// 增长率假设，真实主因（在 `g` 之后）是要求回报假设。
///
/// 乐观档**不**反向下调折现率：那等于用一个无依据的「风险下降」假设去抬高上界，
/// 让乐观档靠折现率灌水而非靠经营改善 —— 区间上界应只由经营假设决定。
const RISK_STRESS_SPREAD: f64 = 0.01;
pub const FORECAST_YEARS: i32 = 5;

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
//   ③ ~~预测期收缩却靠永续正增长撑估值~~ —— **2026-09-23 已撤销**。
//      该矛盾本来就不是「判据能治的病」：它由「永续增长率符号一致性约束」**直接修复**，
//      而残余的「终值占比高」是**全市场统一参数的结构属性**（非逐样本缺陷）
//      ⇒ 当判据用必然零区分力（命中集 ≈ {增长率 ≥ 0}）。详见 ③ 原实现处注释。
//      该属性改为**模型层面的局限声明**（`dashboard_report.rs` 的区间口径文案）。
//   ④ 亏损 + 当期 FCF 收益率极低 —— 公司尚未盈利，当期现金流不具定价意义
//      （2026-09-21 新增，补 ② 的符号缺口：② 的两条形态都预设「净利 > 0」。
//       688114 净利 −2.22 亿、FCF 收益率 1.18% ⇒ 修复前落 `applicable = true`）
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

/// 判据 ④：亏损公司的 FCF 收益率下限（小数）。
///
/// ## 为什么需要这条
///
/// 判据 ② 的两条形态都挂在 `net_profit.filter(|v| *v > 0.0)` 之下 ⇒
/// **公司净利为负时整段判据短路**。后果是**反向不公**：净利为正但现金流差的
/// 公司（601166 兴业银行、300308 中际旭创）被拦下并判 `applicable = false`，
/// 而**真亏损**的公司反而拿到 `applicable = true` —— 净利更差、更该质疑
/// 「拿当期 FCF 折现」这件事的标的，判据对它完全失明。
///
/// 实测对照（2026-09-21 同日两次运行，模板 v70/v73）：
///
/// | 标的 | 净利 | FCF/净利 | `applicable` |
/// |---|---|---|---|
/// | 300308 中际旭创 | +204 亿 | 0.14（判据 ② 命中） | `false` ✅ |
/// | 688114 华大智造 | **−2.22 亿** | 判据 ② 短路 | `true` ❌ |
///
/// 688114 的 DCF：`fcf_anchor = 3.588 亿`、市值 303.87 亿 ⇒ 收益率 **1.18%**，
/// 三档 16.33 / 24.23 / 38.84 元（现价 73.11，`upsidePct = −66.9`），
/// 以 `is_fallback_anchor = false` **全额权重**进 f5（`dcf_anchor_decay = 1.0`），
/// 并被 `value-investor` 按 prompt 指示「直接引用」为 `intrinsic_value_range`
/// ⇒ `margin_of_safety = −201.7%`、`buffett_verdict = 【减持】`。
///
/// ## 判据形态（两条并列，缺一不可）
///
/// - 当期净利 ≤ 0 —— 公司尚未证明其商业模式能产生利润；
/// - 锚定 FCF / 市值 < 本阈值 —— 当期现金流**不具定价意义**。
///
/// ⚠️ **不能**简化成「净利 ≤ 0 即不适用」：那会误伤「一次性减值致亏、
/// 但经营现金流充沛」的正常公司（FCF 收益率 20% 时 DCF 完全成立），
/// 也会与 V74「周期底部用历史正净利归一化锚定」的设计直接冲突。
///
/// ## 取值依据
///
/// 正常经营企业 FCF 收益率 3–8%（折旧与资本开支大致抵消时接近净利口径）；
/// 折现率 10% 意味着「零增长 + 零再投资」的理论收益率下限约 10%。
/// 取 3% 已属**宽松**，只拦「相对市值小到不具备定价意义」的极端形态 ——
/// 688114 的 1.18%（市值/FCF ≈ 85 倍）正是此类。
const FCF_YIELD_INAPPLICABLE_MIN: f64 = 0.03;

// 【**2026-09-23 已撤销**】此处原有判据 ③ 的阈值常量 `TERMINAL_RATIO_MAX = 0.7`，
// 已随该判据一起删除 —— 终值占比**不再**作为适用性判据。论证见 `compute_dcf` 内
// 「③ 【已撤销】」段。一句话：`tvr > 0.7` 的**命中集 ≈ {预测期增长率 ≥ 0}**，
// 与它声称的语义（「结论由永续假设独裁」）无关 ⇒ 零区分力；根因是把**全市场统一的
// 参数常量**（`d − p`）当成了**逐样本的适用性判据**。该属性改为**模型层面的局限声明**
// （见 `dashboard_report.rs` 的区间口径文案）；`terminal_value_ratio` 字段保留供诊断。

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
        self.bond_yield.unwrap_or(DEFAULT_BOND_YIELD)
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

/// 当期自由现金流（TTM 口径，元）—— `compute_dcf` 的 ① 分支与
/// `compute_owner_earnings` 的共同输入。
///
/// ## 为什么需要它（2026-09-21，300308 中际旭创实证）
///
/// vendor 的 `financials[0]` 是**最新报告期**，而中报/季报的
/// `operating_cash_flow` / `capital_expenditure` 是**年内累计值**（与 `eps`、`roe`
/// 同口径，见 [`annualized_eps`] 的 P1-A 记录）。直接把半年累计值当年度值代入，
/// 锚点会被腰斩约一半。
///
/// 更根本的是：在本函数出现之前，这条分支在**生产上从未执行过一次** ——
/// 主要 vendor（eastmoney / akshare / sina / baidu_stock / neodata）长期把这三个
/// 字段硬编码为 `None` ⇒ `direct_fcf ≡ None` ⇒ 锚点永远走 5 年年报均值 fallback，
/// 且适用性判据 ② 永远无法命中（详见 `vendors/eastmoney.rs` 的现金流补充块）。
///
/// ## 取值链（缺数即 `None`，**不得**把缺失当 0）
///
/// ① 年报口径本身即年度值 → `OCF − capex`，缺则退回 vendor 直供的 `free_cash_flow`；
/// ② 中报/季报 → TTM = 「上年年报 + 本期累计 − 上年同期累计」（三段均按 ① 相减）；
/// ③ 还原不出（缺任一段）→ 退回 vendor 直供的 `free_cash_flow`；
/// ④ 全缺 → `None`。
///
/// ```text
/// 实测 300308（东财，2026-09-21）:
///   2025FY   OCF 108.96 亿 − capex 27.60 亿 = +81.36 亿
///   2026H1   OCF  18.00 亿 − capex 48.02 亿 = −30.02 亿   （半年累计）
///   2025H1   OCF  32.18 亿 − capex  9.54 亿 = +22.65 亿
///   ⇒ TTM  = 81.36 + (−30.02) − 22.65 = +28.69 亿（为正 ⇒ 走 ① 分支）
/// ```
/// 若不还原，直接用 2026H1 的 −30.02 亿，就会把一家当期 FCF 为正的公司判成
/// 「当期FCF≤0」并触发历史均值锚定 —— 这正是本函数要消除的错误面。
fn ttm_fcf(financials: &[FinancialReport]) -> Option<f64> {
    let latest = financials.first()?;
    let from_ocf_capex = |f: &FinancialReport| -> Option<f64> {
        Some(f.operating_cash_flow? - f.capital_expenditure?)
    };

    // ① 年报口径
    if latest.report_date.contains("-12-31") {
        return from_ocf_capex(latest).or(latest.free_cash_flow);
    }

    // ② 中报/季报 → TTM 还原
    let ttm = (|| {
        let key = report_period_key(&latest.report_date)?;
        let year = key.get(..4)?.parse::<i32>().ok()?;
        let month_day = key.get(4..)?;
        let prev_year = year.checked_sub(1)?;
        let find = |target: &str| {
            financials.iter().find(|r| report_period_key(&r.report_date) == Some(target))
        };
        let annual = from_ocf_capex(find(&format!("{prev_year}-12-31"))?)?;
        let prev_same = from_ocf_capex(find(&format!("{prev_year}{month_day}"))?)?;
        Some(annual + from_ocf_capex(latest)? - prev_same)
    })();

    // ③ 还原不出 → vendor 直供兜底
    ttm.or(latest.free_cash_flow)
}

/// 当期归母净利润（TTM 口径，元）—— 与 [`ttm_fcf`] 同构。
///
/// 用于 `compute_owner_earnings` 的最后兜底（`净利 × 0.85~0.95` 代理）。
/// 修复前该兜底直接取 `financials[0].net_profit`，在中报口径下是**半年累计**，
/// 于是「所有者收益」与同一份输出里按 TTM 计算的 PE / EPS 口径不一致
/// （与 [`annualized_eps`] 记录的 601166 ROE 口径事故同源）。
///
/// ## ⚠️ 2026-09-23 补回落：原实现**漏了 [`ttm_fcf`] 的 ③ 分支**，造成 fail-open
///
/// 原实现只有 ①②，**还原不出就返回 `None`**；而 `ttm_fcf` ③ 会回落到
/// `latest.free_cash_flow`。两者不对称 ⇒ 序列缺「上年年报 / 上年同期」时，
/// **FCF 侧有值、净利侧 `None`** ⇒ 下游 `ttm_net_profit(financials).filter(...)`
/// 的闭包整段短路 ⇒ **判据 ②（FCF 与净利背离）与判据 ④（亏损 + FCF 收益率过低）
/// 一起静默失明**，`applicability_signals` 恒为空。
///
/// 为什么这是「结论变垃圾」而非「少一条提示」：这两条正是**杠杆畸高 / 亏损且现金流
/// 不具定价意义**的标的唯一的退出通道 ⇒ 失明后这类标的的 DCF 腿会以全额权重喂给
/// `f5`，产出「估值极低」的假信号。**算不出来 ≠ 没问题**（fail-open）。
///
/// 暴露它的不是评审而是测试：`dcf_inapplicable_for_leveraged_negative_fcf_shape`
/// （序列只有年报 + 本期中报，缺上年同期）与
/// `dcf_inapplicable_for_loss_making_with_tiny_fcf_yield`（单期序列）
/// 双双 `signals=[]` ⇒ 见 `cargo test -p axagent-astock-data`。
///
/// ## 残余不对称（已知，未消除，方向偏保守）
///
/// 两函数的「能否还原」判据含**指标自身**的缺失：若 `operating_cash_flow` /
/// `capital_expenditure` 缺失（FCF 侧回落单期）而 `net_profit` 三期齐全（净利侧为 TTM），
/// 则 `ratio = 单期FCF / TTM净利` 被**偏小** ⇒ 判据 ② 更容易命中 ⇒ 偏向「退出该腿」，
/// 方向保守。反向组合（净利缺、FCF 全）在 vendor 载荷里不出现（净利与现金流同行下发）。
fn ttm_net_profit(financials: &[FinancialReport]) -> Option<f64> {
    let latest = financials.first()?;

    // ① 年报口径：直接可用
    if latest.report_date.contains("-12-31") {
        return latest.net_profit;
    }

    // ② 中报/季报 → TTM 还原（与 `ttm_fcf` ② 逐字同构）
    let ttm = (|| {
        let key = report_period_key(&latest.report_date)?;
        let year = key.get(..4)?.parse::<i32>().ok()?;
        let month_day = key.get(4..)?;
        let prev_year = year.checked_sub(1)?;
        let find = |target: &str| {
            financials
                .iter()
                .find(|r| report_period_key(&r.report_date) == Some(target))
                .and_then(|r| r.net_profit)
        };
        let annual = find(&format!("{prev_year}-12-31"))?;
        let prev_same = find(&format!("{prev_year}{month_day}"))?;
        Some(annual + latest.net_profit? - prev_same)
    })();

    // ③ 还原不出 → **回落到最新期单期值**（与 `ttm_fcf` ③ 同构）。
    //    回落而非返回 None：判据 ② 的分子走 `ttm_fcf`，若此处返回 None 则两侧
    //    可用性不一致 ⇒ 判据整体失效（见上文 fail-open）。回落后两侧**同口径**
    //    （要么都 TTM、要么都单期）。
    ttm.or(latest.net_profit)
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
    /// **我方采集缺陷**标记：当期 FCF 缺失的原因是**数据源未提供**，而非标的现金流为负。
    ///
    /// 2026-09-21 新增（审计 `AUDIT-dcf-cashflow-anchor-2026-09-21.md` §6.4 / §6.10.5 拍板项）。
    /// 与 `is_fallback_anchor` **正交**：后者表达「锚是历史代理」（两态都为 `true`），
    /// 本字段表达「**为什么**用代理」——
    ///   · `true`  = 现金流量表数据缺失 ⇒ **我方取数失败**，属应上报的缺口；
    ///   · `false` = 当期 FCF 真为负     ⇒ 标的经营状态，**不是**缺口（不得上报）。
    ///
    /// 为什么不让下游按 `basis` 文案判分支：`basis` 是给人与 LLM 看的诊断文本，
    /// 文案一改判据就**静默失效** —— `is_fallback_anchor` 当初正是为此从
    /// `== FCF_FALLBACK_BASIS` 的等值比较改成布尔量（见下方 P0-I 注释）。
    /// 本字段沿用同一纪律。三态区分本身由测试 `dcf_basis_tells_three_states_apart` 钉住。
    ///
    /// 消费点：`data-quality.rhai` 的 `upstream_data_gaps`（**只告警、不扣分**，
    /// 不进 `pm_compute_factor_completeness` 分母 —— 避免「列表长度 ≠ 公式分母」）。
    fcf_data_missing: bool,
    /// **模型适用性**：DCF 的前提假设是否对本标成立（2026-09-14 新增）。
    ///
    /// `false` 表示「本标的不满足 DCF 的前提」—— 数值仍会算出（保持零破坏性，
    /// 下游有旧模板依赖三档值），但**下游不得把它当作可靠估值证据**。
    ///
    /// 判据锚定**数据形态**而非行业标签，详见模块顶部常量区的说明。命中任一即 `false`：
    /// ① `debt_ratio > LEVERAGE_INAPPLICABLE_PCT(80)` —— 净利由杠杆驱动
    /// ② 净利为正但当期真实 FCF ≤ 0（符号相反），或 `0 < FCF/净利 < 0.3`（量级脱钩）
    /// ③ ~~终值现值占比 > 0.7~~ —— **2026-09-23 撤销**（命中集 ≈ {增长率 ≥ 0}，零区分力）
    /// ④ 当期净利 ≤ 0 **且** 锚定 FCF / 市值 < `FCF_YIELD_INAPPLICABLE_MIN(3%)`
    ///    —— 公司尚未盈利且当期现金流不具定价意义（2026-09-21 新增）
    /// ⑤ 配置的永续增长率与折现率利差 < `MIN_TERMINAL_SPREAD(1.5pp)`
    ///    —— 终值分母触及地板，估值由差值决定（**2026-09-23 新增**）。
    ///    ⚠️ ⑤ 的触发源与 ①–④ **不同类**：①–④ 判**标的的数据形态**，⑤ 判
    ///    **调用方配置**是否把模型推进发散区。放在同一出口是因为对下游后果相同
    ///    （该腿数值不可信 ⇒ 必须整体退出）。
    ///    ⚠️ ③ 撤销后，「终值占比高」这一**全市场共有**的结构属性由 ⑤ 的**配置侧**
    ///    条件承接（利差可配、可跨运行不同）——语义重心从「标的不好」移到「配置发散」。
    ///
    /// ⚠️ ④ 补的是 ② 的**符号缺口**：② 的两条形态都写在
    /// `net_profit.filter(|v| *v > 0.0)` 之内 ⇒ 净利为负时整段短路，
    /// 于是「净利为正但现金流差」被拦、「真亏损」反被放行（**反向不公**）。
    /// 详见常量 `FCF_YIELD_INAPPLICABLE_MIN` 的文档。
    ///
    /// ## 601166 实证（2026-09-14，样本 86c7d441）
    ///
    /// 现价 18.15 元，DCF 给 `low/mid/high = 40.17/44.13/49.26`（`upsidePct = 143.1`），
    /// 而 LLM 层 `llmAction = 观望` / `positionPct = 0` —— 同一份输出里两个结论互相打脸。
    /// 复算 `mid = 44.13` 精确复现，其中 70.6% 来自永续终值，
    /// 且 `growth = −0.25%` 与 `perpetual_growth = +3%` 并存（模型自相矛盾）。
    ///
    /// 本字段为 `false`，命中 **①（91.6% > 80%）与 ②（净利为正、当期 FCF ≤ 0）两条**。
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
    current_price: f64,
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
    // P0-I(2026-09-12): fallback 锚定的口径文案提为常量。
    //
    // 2026-09-21 拆成**两条**：原实现只有一条，且文案写死「（周期底部）」，
    // 把「FCF 真为负」与「现金流量表数据缺失」两种**性质完全不同**的情形
    // 合并成同一句诊断：
    //   · 真为负（`Some(v) if v <= 0`）= 标的当期现金流为负 → 标的属性；
    //   · 缺失（`None`）            = 我们没取到数 → **我方采集缺陷**。
    // 实测后果（300308，样本 ee770189）：对一家营收 +182.5%、ROE 62.6% 的公司
    // 断言「周期底部」，且该句被 value-investor 原样引用进 `risk_flags`
    // （「当期FCF≤0，DCF基于归一化锚定，绝对估值锚偏低」）⇒ 假诊断流入结论。
    // 两态**处置相同**（都用历史代理锚），但诊断必须说真话。
    const FCF_FALLBACK_BASIS: &str = "当期FCF≤0，改用近5年报正净利均值×0.90归一化锚定";
    const FCF_MISSING_BASIS: &str =
        "现金流量表数据缺失（该数据源未提供 OCF/资本开支），改用近5年报正净利均值×0.90归一化代理锚定";
    // 2026-09-14：`direct_fcf` 提到外层作用域 —— 适用性判据 ②（`FCF/净利` 背离）
    // 需要看到**当期真实** FCF，而不是 fallback 后的代理值（代理值恒 ≈0.9×净利，
    // 会把「符号相反」这个最强信号抹掉）。
    // 2026-09-21：改走 [`ttm_fcf`]（TTM 还原 + vendor 直供 FCF 双通道）。
    //   原实现读 `financials[0]` 的**年内累计值**，中报口径下 OCF 只有半年；
    //   且因 vendor 侧恒不提供这三列，本变量在生产上**一直是 `None`**。
    let direct_fcf = ttm_fcf(financials);
    let (fcf, fcf_basis) = match direct_fcf {
        Some(v) if v > 0.0 => (v, "当期FCF".to_string()),
        Some(_) => match normalized_annual_profit(financials, 5) {
            Some(avg_np) => (avg_np * 0.90, FCF_FALLBACK_BASIS.to_string()),
            None => {
                return (
                    None,
                    "当期FCF≤0且近5年报无正净利年度（持续亏损），DCF模型不适用".to_string(),
                    None,
                )
            },
        },
        None => match normalized_annual_profit(financials, 5) {
            Some(avg_np) => (avg_np * 0.90, FCF_MISSING_BASIS.to_string()),
            None => {
                return (
                    None,
                    "现金流量表数据缺失且近5年报无正净利年度，DCF模型不适用".to_string(),
                    None,
                )
            },
        },
    };
    // P0-I(2026-09-12): 锚定来源标记，随 `DcfAssumptions` 落库，供 f5 做置信度衰减。
    // 2026-09-21：判据由「等值于某一条 fallback 文案」改为「**不是**当期真实 FCF」
    //   —— 语义没变（该布尔量表达的就是「锚定是历史代理」），但两态 fallback
    //   现在都正确标记为 true。原实现用 `== FCF_FALLBACK_BASIS` 等值比较，
    //   拆分文案后若不改，缺失态会被**静默**标成 false ⇒ f5 的衰减门（V77）
    //   会在「数据缺失」这一最需要衰减的路径上失效。
    let is_fallback_anchor = fcf_basis != "当期FCF";
    // 2026-09-21：与 `is_fallback_anchor` **正交**的第二判别 —— 只在「缺数」态为 true。
    //   与上面那条同源使用**常量**（不是字面量）比较：两处引用同一 `const`，文案再改也不会漂移。
    //   注意**不能**用 `fcf_basis.contains("缺失")` 之类的子串匹配去替：那只是把等值比较
    //   换成更宽的文本匹配，仍然把「判据」绑在给人看的文案上。
    let fcf_data_missing = fcf_basis == FCF_MISSING_BASIS;
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

    // ── `p` 的可行域守卫（2026-09-23；两条约束合并，此前只有利差那一条）──────────
    //
    // 病根：`p` 与 `d` **都来自用户可配的扁平参数**（`dcf_perpetual_rate` /
    //   `dcf_discount_rate`），而 `pct()` 的守卫只有 `0 < raw ≤ 100`
    //   ⇒ `p ≥ d` 是可配出来的。此时终值 `= FCF₅(1+p)/max(d−p, 地板)` 的**分母塌到地板**
    //   ⇒ 终值被放大到 `FCF₅(1+p)/0.001`（`p = 100%` 时约 **2000 倍**，正常利差下约 15 倍），
    //   **静默无日志**。
    // `MAX_PERPETUAL_GROWTH` 的文档写着「不变量：必须 < DISCOUNT_RATE」—— 那条不变量
    //   只约束**两个常量之间**的关系，对配置通道**没有任何约束力**，这正是缺口所在。
    //
    // 处置：钳到可行域上界（见下），并把钳位**上报**为一条适用性信号。
    //   为什么上报而非静默钳：被钳过的估值由「上界」而非由配置假设决定，属**模型前提不成立**，
    //   与判据 ①②④ 同级。`applicable` 由 `applicability_signals.is_empty()` 派生
    //   ⇒ 该腿自动退出，不会带着一个伪造的数值流入 f5。
    // 顺序：本约束在「符号一致性约束」**之后** —— 后者处理的是语义矛盾（负增长配正永续），
    //   本条处理的是**数值发散**，两者独立，先后不影响结果（取 min 的组合是交换的）。
    // ── `p` 的可行域上界 = 两条约束取更紧者（2026-09-23 合并）──
    //
    // 约束 A（利差）：`d − p ≥ MIN_TERMINAL_SPREAD` ⇒ `p ≤ d − 1.5pp`
    // 约束 B（无风险利率）：`p ≤ MAX_PERPETUAL_GROWTH = r_f`
    //
    // ⚠️ 合并前，约束 B **只出现在 `high_perpetual` 的 `.min()` 里**，基准档不受它管 ⇒
    //   用户把 `dcf_perpetual_rate` 配成 5%（`pct()` 只守 `0 < raw ≤ 100`）时 `mid`
    //   **直接采用 5%**，越过模型自己申报的 `r_f = 1.7%`，**静默无日志**。而这一档正是
    //   `f5` 估值因子的输入。实测（300642 参数，`d = 7.7%`）：`p = 5%` ⇒ `mid = 40.24`
    //   （+30.7%）、`tvr` 升到 83.2% ⇒ 结论几乎完全由一条违反前提的假设决定。
    //
    // ⚠️ 为什么必须记 `spread_is_binding`：两条约束的**数值量级差两个数量级**
    //   （`d − 1.5pp ≈ 6.2pp` vs `r_f = 1.7pp`）⇒ 默认参数下 **B 恒为紧约束**，
    //   A 只在 `d < 3.2%` 时才紧。上报时若不分清是哪条被违反，就会印出
    //   「利差不足，已钳至 6.2%」而实际生效值是 1.7% —— **上报值本身撒谎**。
    //   故上报文案同时给出两条约束与**生效值**，让读者能自行判断（见判据 ⑤）。
    let max_perpetual_by_spread = (discount_rate - MIN_TERMINAL_SPREAD).max(MIN_PERPETUAL_GROWTH);
    let spread_is_binding = max_perpetual_by_spread < MAX_PERPETUAL_GROWTH;
    let max_perpetual_allowed = max_perpetual_by_spread.min(MAX_PERPETUAL_GROWTH);
    let perpetual_clamped_by_config = perpetual_growth > max_perpetual_allowed + f64::EPSILON;
    let perpetual_growth = perpetual_growth.min(max_perpetual_allowed);

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
        // 2026-09-23：地板由裸字面量 `0.001` 换为具名常量 `MIN_TERMINAL_SPREAD`（1.5pp）。
        // 正常配置下**不可达**（`p` 已被 `max_perpetual_by_spread` 钳到 `d − 地板` 之下），
        // 故本行是纯粹的第二道保险 —— 若它真的生效，说明前面那道守卫被绕过。
        let terminal_spread = (d - p).max(MIN_TERMINAL_SPREAD);
        let terminal_value = terminal_fcf / terminal_spread;
        let terminal_pv = terminal_value / (1.0 + d).powi(forecast_years);
        (pv + terminal_pv, terminal_pv)
    };

    // 悲观情景：增长率**向悲观方向**缩放，永续增长率打 7 折，**要求回报 +1pp**。
    //
    // 2026-09-23 起本档语义由「单参数敏感性下界」改为**悲观情景**：
    //   · 原实现只缩放 `g` 与 `p`，把弹性**最大**的 `d` 留在常量上 ⇒ 该档自称
    //     「保守下界」，实际只是「错过主因」的角落点；
    //   · 弹性实测（300642，**终态参数** `p = 1.3% d = 7.7%`；脚本 `sci21`）：
    //     `|E_d| = 1.28` > `E_g = 0.70` > `E_p = 0.17`
    //     —— `d` 上浮 1pp 使估值变动 1.40 倍，**大于** `p` 整个 ×0.7~×1.3 档的 1.11 倍。
    //     ⚠️ 早期的 `−1.96 / 0.78 / 0.71`（`p = 4%` 时测）不得再引用：`E_p ∝ p/(d−p)²`
    //     不是常数，`p` 降下来后 `E_p` 塌得更快，旧数字会夸大 `p` 的地位。
    //   ⇒ 悲观情景应当**同时**要求：经营假设不利 **且** 要求回报上升。
    //
    // 本档是 `upsidePct`（面板「上行空间」）的基准 —— 即**安全边际**口径：
    // 「即便按悲观情景重估，现价仍低于该值吗」。
    let low_growth = if growth >= 0.0 {
        growth * LOW_GROWTH_SCALE_POS
    } else {
        growth * LOW_GROWTH_SCALE_NEG
    };
    let low_perpetual = (perpetual_growth * LOW_PERPETUAL_SCALE_POS).max(MIN_PERPETUAL_GROWTH);
    let low_discount_rate = discount_rate + RISK_STRESS_SPREAD;
    let (low, _) = dcf_two_stage(fcf_per_share, low_growth, low_perpetual, low_discount_rate);

    // 基准情景：原始增长率（已 clamp 到 `[min_growth, max_growth]`）与永续增长率。
    // ⚠️ 本档**不是**内在价值的点估计 —— 它只是「基准假设下的值」。区间非概率区间
    //   ⇒ 从区间里挑任何一档当点估计都是任意的（此处正是原 `upsidePct` 的错源）。
    let mid_growth = growth;
    let (mid, mid_terminal_pv) =
        dcf_two_stage(fcf_per_share, mid_growth, perpetual_growth, discount_rate);

    // 乐观情景：增长率**向乐观方向**缩放，永续增长率放大 1.3 倍；折现率保持基准
    //   （见 `RISK_STRESS_SPREAD` 文档：上界不得靠下调要求回报灌水）。
    //
    // ⚠️ 2026-09-23：上界由 `max_growth` 改为 `max_growth × HIGH_GROWTH_SCALE_POS`。
    //   原实现两者共用同一上界 ⇒ `growth ≥ 20%` 时 `growth × 1.5 ≥ 30%` **被砍回中性档**
    //   （`growth = 30%` 时实际乘子 = 1.0），而面板文案仍声称「×1.5」⇒ **口径与实算不符**。
    //   全库 6 条高终值占比样本里，**2 条** `growth` 正好顶在 30%（其中一个被完全压平）。
    //   两个上界职责不同：`max_growth` 围的是**基准预测**（有当期经营数据支撑），
    //   本处上界围的是**按乘子机械展开的情景**，故应是前提上界 × 该乘子。
    let max_growth_high = max_growth * HIGH_GROWTH_SCALE_POS;
    let high_growth = (if growth >= 0.0 {
        growth * HIGH_GROWTH_SCALE_POS
    } else {
        growth * HIGH_GROWTH_SCALE_NEG
    })
    .clamp(min_growth, max_growth_high);
    // 乐观档永续同样受两条上限约束（利差 / `r_f`）—— 后者由模块级编译期断言
    // `PERPETUAL_GROWTH × HIGH_PERPETUAL_SCALE_POS ≤ MAX_PERPETUAL_GROWTH` 保证
    // **不会静默砍掉乘子**。此处保留 `.min()` 是因为 `p` 仍可经扁平参数被配到更大值，
    // 而那时的越界由判据 ⑤ 上报（不会静默）。
    let high_perpetual = (perpetual_growth * HIGH_PERPETUAL_SCALE_POS)
        .min(MAX_PERPETUAL_GROWTH)
        .min(max_perpetual_by_spread);
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
    //
    // 2026-09-23 口径收敛：净利改走 [`ttm_net_profit`]，**不再读 `latest.net_profit`**。
    //
    // 病根：同一判据的两侧口径不一致 —— 分子 `direct_fcf` 走 [`ttm_fcf`]（TTM 还原），
    //   分母却是 `latest.net_profit`（**最新期累计值**，中报口径下只有半年）。
    //   而 `ttm_net_profit` 早在 `compute_owner_earnings` 的兜底分支里就用过，
    //   其文档注释记录了同源事故（「修复前直接取 financials[0].net_profit，在中报
    //   口径下是半年累计 ⇒ 与同一份输出里按 TTM 计算的 PE/EPS 口径不一致」）
    //   —— **只是判据侧漏接了**。
    //
    // 为什么是方向性偏差而非精度问题：中报口径把分母腰斩 ⇒ 比值被系统性放大 2 倍 ⇒
    //   · 上侧（现金流远超盈利）被**虚假放大**；
    //   · 下侧 `ratio < FCF_NP_DIVERGENCE_MIN` 本该拦下的「现金流跟不上盈利」样本
    //     反而因分母偏小而**越过阈值**（假阴性）—— 这正是本条判据存在的理由。
    //
    // 实证 300642（报告期 2026-06-30）：`latest.net_profit` = 1047 万（半年累计），
    //   与 TTM FCF 1.352 亿相比 ratio = 12.92 ⇒ 旧口径下背离被夸大 2 倍以上。
    if let Some(np) = ttm_net_profit(financials).filter(|v| *v > 0.0) {
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

    // ③ 【2026-09-23 **撤销**】终值占比不再作为适用性判据 —— 它是**结构量**，不是样本特征
    //
    // ## 演化史（本判据被改了三次，前两次都失效）
    //
    // ① 诞生实现：`growth < 0.0 && tvr > 0.7`（601166 样本）⇒ 生产上**零命中**；
    // ② 2026-09-23 上午：删掉 `growth < 0.0` 合取项（判据「零命中」归因于它）
    //    ⇒ 变成**恒命中**；
    // ③ 2026-09-23 同日下午：复算 `tvr` 关于 `g` 的曲线后**整体撤销**（本次）。
    //
    // ## 复算证据（`output/sci7-tvr-threshold.mjs`；d = 8.5%、5 年预测期）
    //
    // | 预测期增长率 g | tvr（p = 4%） | tvr（p = 2%） | ③ 是否命中 |
    // |---|---|---|---|
    // | −30%      | 44.9% | 44.9% | 否 |
    // | −5%       | 63.9% | 63.9% | 否 |
    // | **0%**    | 79.6% | 72.6% | **是** |
    // | +10%      | 82.6% | 76.3% | 是 |
    // | +30%      | 86.5% | 81.3% | 是 |
    //
    // ⇒ **临界点落在 `g > 0` 上，与 `p` 取 2% 还是 4% 无关**（`p` 从 2% 到 4% 都得出
    //   同一个临界值）。机理是结构性的：
    //     · `g < 0` ⇒ 「永续增长率符号一致性约束」把 `p` 压到 0 ⇒ 终值倍数塌到
    //       `1/d = 11.8×` ⇒ tvr 落 44.9%–63.9%；
    //     · `g ≥ 0` ⇒ `p` 生效 ⇒ 终值倍数 `1/(d−p) = 15.7×–23.1×` ⇒ tvr ≥ 72.6%。
    //   于是 `tvr > 0.7` 的**命中集 ≈ {g ≥ 0}** —— 它实际在判「预测期非衰退」，
    //   与它声称的语义（「结论由永续假设独裁」）**毫无关系**。零区分力。
    //
    // ## 更深一层：本判据的**思路**不成立（这才是撤销的真正理由）
    //
    // 「估值有多依赖永续假设」由终值倍数 `1/(d − p)` 决定，而 `d` 与 `p` 是
    //   **全市场统一的常量** ⇒ 在参数统一的前提下，该属性是**所有标的共有**的，
    //   不是某些标的的缺陷。拿它做**逐样本适用性判据**，等于用全局常量去否定逐标的
    //   估值 —— **作用域错配**（与「拿行业标签当数据形态判据」同族的错误）。
    //   ⇒ 它只应作为**模型层面的局限声明**：见 `dashboard_report.rs` 的区间口径文案
    //     （已声明「本模型 5 年预测期 + 该折现率下，非衰退标的的估值有 ≥73% 来自
    //     永续终值」），以及保留的 `terminal_value_ratio` 字段（诊断用）。
    //
    // ## 撤销的代价与不撤销的代价（为什么必须撤）
    //
    // · 不撤销：`g ≥ 0` 的样本（占绝大多数，含全部优质成长标的）其 DCF 腿会被
    //   **整体剔除**（`applicable = false`）⇒ DCF 在 f5 中实质失效。
    //   即「用一个全市场统一的结构属性，把几乎所有标的的估值证据关掉」。
    // · 撤销后用户实证的 300642 矛盾**仍已解决**，且不是靠剔除腿：`upsidePct`
    //   已由 `mid` 改为**悲观档**基准 ⇒ 300642 从「+92.8% 低估」变为
    //   「**−14.2%**（现价高于悲观档 = 无安全边际）」⇒ 与决策「观望」自洽。
    //   ⇒ 剔除腿只是把「一个错误结论」换成「没有结论」，属回避而非修复。

    // ④ 当期亏损 + 锚定 FCF 收益率极低：市场不按当期现金流定价（2026-09-21 新增）
    //
    // 本条补的是判据 ② 的**符号缺口**：② 的两条形态（符号相反 / FCF/净利 < 0.3）
    // 都写在 `net_profit.filter(|v| *v > 0.0)` 之内，净利为负时整段短路
    // ⇒ 亏损公司（恰恰最该质疑 FCF 折现前提）反而落 `applicable = true`。
    // 完整量化与同日对照见常量 `FCF_YIELD_INAPPLICABLE_MIN` 的文档注释。
    //
    // 判据锚定**数据形态**（净利符号 + FCF 相对市值），不锚定行业标签 ——
    // 与模块顶部「不能写 if 银行 then 跳过」的约束同源。
    //
    // 缺数（`net_profit == None`）**不**命中：与判据 ② 同口径，缺数据 ≠ 模型不成立，
    // 该情形由 `is_fallback_anchor` 承担置信度衰减。同理 `current_price` 或
    // `shares` 非正时不命中（无市值 ⇒ 无法判断相对规模，不得凭空判定不适用）。
    // 2026-09-23：净利口径与判据 ② 统一走 [`ttm_net_profit`]。
    //   本条判的是「净利符号」——中报口径下半年净利与 TTM 净利的**符号**可能相反
    //   （上年年报大额亏损 + 本期转正 ⇒ 半年为正、TTM 仍为负），此时旧口径会
    //   把「尚未证明商业模式能盈利」的公司误判成已盈利 ⇒ 本条判据整体失明，
    //   而它正是为「净利为负时判据 ② 短路」这个缺口而生的。缺数（`None`）不命中，
    //   与判据 ② 同口径（缺数据 ≠ 模型不成立）。
    if let Some(np) = ttm_net_profit(financials).filter(|v| *v <= 0.0) {
        let market_cap = current_price * shares;
        if market_cap > 0.0 {
            let fcf_yield = fcf / market_cap;
            if fcf_yield < FCF_YIELD_INAPPLICABLE_MIN {
                applicability_signals.push(format!(
                    "当期净利 {:.2} 亿 ≤ 0 且锚定 FCF 收益率仅 {:.2}%（FCF {:.2} 亿 / 市值 {:.2} 亿）\
                     < {:.0}%：公司尚未盈利且当期现金流不具定价意义，DCF 口径不成立",
                    np / 1e8,
                    fcf_yield * 100.0,
                    fcf / 1e8,
                    market_cap / 1e8,
                    FCF_YIELD_INAPPLICABLE_MIN * 100.0
                ));
            }
        }
    }

    // ⑤ 配置的永续增长率超出**模型可行域**（**配置可得**，非数据形态）—— 2026-09-23 新增
    //
    // 本条的触发源与前四条不同：①②③④ 判的是**标的的数据形态**，本条判的是
    //   **调用方给的配置**是否把模型推进发散区。放在同一个 `applicability_signals`
    //   出口，是因为对下游而言后果相同 —— 该腿数值不可信，必须整体退出。
    // 判据与「谁配的」无关：只要 `configured_perpetual_growth` 越过可行域上界
    //   （= `min(d − MIN_TERMINAL_SPREAD, MAX_PERPETUAL_GROWTH)`），估值就由被钳后的值
    //   而非由配置假设决定。
    //
    // ⚠️ 文案必须给出**两条约束 + 生效值**，不能只报被违反的那一条：
    //   约束 A（利差 `d − 1.5pp`）与约束 B（`r_f`）量级差两个数量级 ⇒ 默认参数下
    //   B 恒紧。若只按「利差距离不足」措辞，会印出「已钳至 6.2%」而上报字段里
    //   实际是 1.7% —— **上报值自身撒谎**，比不报更糟（诊断时会把责任归到错误的参数上）。
    if perpetual_clamped_by_config {
        applicability_signals.push(format!(
            "配置的永续增长率 {:.2}% 超出模型可行域（上限 {:.2}%，由「{}」更紧地约束）：\
             生效值已钳至 {:.2}%。两条约束为 g_terminal ≤ r_f = {:.2}%（无风险利率）\
             与 d − p ≥ {:.1}pp（终值分母利差）。配置值越界属**配置与模型前提冲突**，\
             与标的质地无关 ⇒ 本配置下的 DCF 数值不可用",
            configured_perpetual_growth * 100.0,
            max_perpetual_allowed * 100.0,
            if spread_is_binding {
                "终值分母利差"
            } else {
                "无风险利率"
            },
            perpetual_growth * 100.0,
            MAX_PERPETUAL_GROWTH * 100.0,
            MIN_TERMINAL_SPREAD * 100.0
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
            fcf_data_missing,
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
///
/// `g` 为未来 7-10 年预期增长率（**百分数**，如 17.78 表示 17.78%）；`Y` 为 AAA
/// 企业债收益率基准（百分数，缺省 4.4）。
///
/// ⚠️ 2026-09-22 量纲修复：修复前本函数把 `g` 当**小数**代入（0.1778），而同式的
/// `4.4 / Y` 用百分数 ⇒ **一个表达式两套口径**。后果：增长项被压 100 倍
/// （乘数 `8.5 + 2×0.1778 = 8.856`，而非 `8.5 + 2×17.78 = 44.06`），
/// 内在价值系统性**低估约 5 倍**（300642 实证：0.67 元 → 3.33 元；DCF mid 26.39 元，
/// 修复前两模型差 39 倍）。
/// 影响面已量化（23 个历史样本）：仅 **2 个**（688114 / 300308，均 g 顶 30% 上界）
/// 格雷厄姆腿符号翻转，其余 21 个因 `g = 0` 完全不受影响。
///
/// ⚠️ `GrahamAssumptions.growth` 对外仍是**小数**口径（与 DCF 的 `MIN_GROWTH` /
/// `MAX_GROWTH` 同源，见下方赋值处），只在**本公式内**换算为百分数 ——
/// 这是两张刻度唯一相遇的地方，改动时不要顺手把 `g` 本身改成百分数。
///
/// V74(2026-09-10): 返回 `Option<f64>`——EPS≤0 且近 5 年报无正 EPS 年度时
/// 返回 None（公式不适用），不再用 0 冒充估值。当期 EPS≤0 但历史存在正 EPS
/// 年报时，用正 EPS 均值归一化（周期底部锚定）。
///
/// P1-A(2026-09-11): EPS 一律经 `annualized_eps()` 取**年度口径**（中报/季报
/// 累计值还原为 TTM）；g 取数从 `profit_yoy` 改为 `revenue_yoy` 并允许负增长。
/// 格雷厄姆公式**实际生效**的假设快照（2026-09-21 新增）。
///
/// ## 为什么要落这份快照
///
/// 与 `dcf.assumptions` 同源动机：只输出 `graham.intrinsicValue` 时，
/// 「这个数怎么来的」只能靠反解，而反解不唯一。
///
/// 实测 300308（现价 926.43，`revenue_yoy = 182.5%`，EPS 18.47，`Y` 取缺省 4.4）：
/// ```text
/// 修复前（g 当小数，量纲错）：168.08  = 18.47 × (8.5 + 2 × 0.30) × 4.4 / 4.4
/// 修复后（g 为百分数）      ：1265.20 = 18.47 × (8.5 + 2 × 30)   × 4.4 / 4.4
///                            └──────── 增长项顶在 MAX_GROWTH 上界 ────────┘
/// ```
/// ⇒ 现价 926.43 下 `upsidePct` 从 **−81.9%** 翻转为 **+36.6%**，腿信号
/// `pm_saturate(36.6, 40) = +0.478`（修复前为 −0.672）。
///
/// ⚠️ **口径**：以上用**精确 EPS 18.47**。若改用落库 `intrinsicValue = 168.08`
/// 反解 EPS（`168.08 / 9.1 = 18.4703`）再重算，得 `1265.22` / `+36.5%` / `+0.463` ——
/// 两组数只差 round 精度，**不是分歧**。引用时须注明基准，勿当作两个矛盾结论。
///
/// 即：`upsidePct` 可被**纯 PE 恒等式**复现 —— `(8.5+2g%)×4.4/Y ÷ PE − 1`，
/// 两边 `EPS` 相消 ⇒ 该输出对高 PE 标的**几乎不含公司特定信息**，
/// 只反映「现价 PE 相对基准 PE 的偏离」。
/// （该恒等性由 **EPS 相消**保证，与 `g` 的量纲无关 ⇒ 2026-09-22 的量纲修复
/// 不改变本段结论，只改变乘数大小。）
///
/// ## 偏差方向（决定处置取向）
///
/// 上界 `MAX_GROWTH = 30%` 是**保守假设**（认为高增速不可持续）。
/// 对真实增速 ≥ 30% 的公司，代入的增长率**低于**实际 ⇒ 内在价值系统性**偏低**
/// ⇒ `upsidePct` 系统性**偏负** ⇒ 看空被夸大。
/// 这与 DCF 侧 `is_fallback_anchor`（代理锚偏低）**同方向**，
/// 故处置也取同族：**软衰减**（下游 f5 的 `valuation_graham_growth_clamped`），
/// 而不是整腿剔除 —— 它仍是与 FCF 无关的格雷厄姆信号，只是假设被顶死。
///
/// 下界方向的偏差相反（内在价值偏高 ⇒ 看多被夸大），单独标记以便将来分别处置。
#[derive(Debug, Clone, Copy, PartialEq)]
struct GrahamAssumptions {
    /// 实际代入公式的增长率（小数形式）。
    growth: f64,
    /// `revenue_yoy` 超过 [`MAX_GROWTH`] ⇒ 增长率被**上限封顶**。
    growth_clamped_upper: bool,
    /// `revenue_yoy` 低于 [`MIN_GROWTH`] ⇒ 增长率被**下限封底**。
    growth_clamped_lower: bool,
    /// `revenue_yoy` 缺失，改用配置缺省增长率（**非**实测值）。
    growth_from_default: bool,
    /// 公式 `4.4 / bond_yield` 修正项实际代入的债券收益率（百分数）。
    bond_yield: f64,
}

fn compute_graham_value(
    financials: &[FinancialReport],
    current_price: f64,
    config: Option<&ValuationConfig>,
) -> Option<(f64, GrahamAssumptions)> {
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
    // 2026-09-21：上下界改用与 DCF 同源的 `MIN_GROWTH` / `MAX_GROWTH`——
    //   两者语义相同（预测期增长率区间），原先 graham 手抄 `-0.30, 0.30`
    //   是同一常量的第二份副本，改一侧不改另一侧即静默分叉。
    let raw_growth = latest.revenue_yoy.map(|y| y / 100.0);
    let g =
        raw_growth.map(|v| v.clamp(MIN_GROWTH, MAX_GROWTH)).unwrap_or_else(|| cfg.default_growth());
    let assumptions = GrahamAssumptions {
        growth: g,
        growth_clamped_upper: raw_growth.is_some_and(|v| v > MAX_GROWTH),
        growth_clamped_lower: raw_growth.is_some_and(|v| v < MIN_GROWTH),
        growth_from_default: raw_growth.is_none(),
        bond_yield,
    };
    // 2026-09-22 量纲修复：原式 `8.5 + 2.0 * g` 里 `g` 传的是**小数**（0.1778），
    //   而同式的 `4.4 / bond_yield` 用的是**百分数**（bond_yield 默认 4.4 = 4.4%）
    //   ⇒ **同一个表达式两套口径**。后果：增长溢价被压 100 倍
    //   （8.5 + 2×0.1778 = 8.856，而格雷厄姆原式要求 8.5 + 2×17.78 = 44.06），
    //   估值被系统性低估约 5 倍。实证 300642：修前 0.67 元（vs DCF mid 26.39 元，
    //   两模型差 39 倍），修后 ~3.34 元。
    //   ⚠️ `assumptions.growth` 对外仍是**小数**口径（与 DCF 的 MIN/MAX_GROWTH 同源，
    //   见上方 clamp），只在**本公式内**换算为百分数 —— 不要改 `g` 本身，
    //   否则会连带污染 assumptions 的展示口径。
    //   ⚠️ `g` 为负时乘数可低于 8.5（负增长惩罚），极端负增长下乘数为负 ⇒ 由
    //   `.max(0.0)` 兜为 0：格雷厄姆式估值对深度衰退股给 0 是原式的固有行为，不是缺陷。
    Some(((eps * (8.5 + 2.0 * g * 100.0) * 4.4 / bond_yield).max(0.0), assumptions))
}

/// 巴菲特所有者收益（元）
/// 注意：vendor 返回的财务数据单位均为"元"，无需缩放
fn compute_owner_earnings(financials: &[FinancialReport]) -> Option<f64> {
    if financials.is_empty() {
        return None;
    }
    // 2026-09-21：改走 [`ttm_fcf`]（TTM 口径 + vendor 直供 FCF 双通道）。
    //   修复前这里是「`financials[0]` 的 OCF − capex」，取的是**年内累计值** ——
    //   中报口径下 OCF 只有半年 ⇒ 所有者收益被腰斩。更严重的是：该分支因
    //   vendor 侧恒不提供现金流而**从未执行过**，一直落到下面的 `净利 × factor`
    //   兜底，于是「所有者收益」实际是**非现金的净利润**，而它被写进
    //   `owner_earnings_yield_pct` 供 LLM 当论据（300308 实测该值 1.3%，
    //   与 PE 倒数 1.9% 同量级 ⇒ 该"指标"不携带净利润以外的新信息）。
    if let Some(fcf) = ttm_fcf(financials) {
        return Some(fcf.max(0.0));
    }
    let f = &financials[0];
    // 兜底：净利 × 负债率折价。净利同样取 TTM 口径，避免与同报告内的 PE/EPS 口径不一致。
    let net = ttm_net_profit(financials).unwrap_or_else(|| f.net_profit.unwrap_or(0.0));
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

/// 算法综合估值档位（`value_signal`）—— 由安全边际 / F-Score / 护城河 / 所有者收益率合成。
///
/// ## 2026-09-21 修复：原评分函数**无法表达「高估」**
///
/// 原实现对 `mos_pct` **只在为正时加分**，为负落 `_ => {}` 加 0 分；
/// 而 F-Score 与护城河两项**恒为正**且合计已达 `9×5 + 100/5 = 45`
/// —— 恰好就是「合理偏低」档的阈值。于是：
///
/// > **只要 F-Score ≥ 6 且护城河 ≥ 75，无论价格多高都稳落「合理偏低」**，
/// > 安全边际为负这件事在评分里**没有任何权重**。
///
/// 实证 300308（2026-09-21，样本 `ee770189`）：现价 926.43、DCF 中性 139.22
/// （`upsidePct = −85.0`）、安全边际 −565.4%，同一条 payload 的 `value_signal`
/// 输出 **「合理偏低」**，与 `margin_of_safety.level`「无（高估风险）」**直接矛盾**。
/// value-investor 在正文里点出了该矛盾（「算法 value_signal 为『合理偏低』，
/// 但绝对估值锚与现价偏离超 80%」）却仍被 prompt 要求「直接引用」
/// ⇒ 矛盾被原样带进结论。
///
/// ## 修法与口径
///
/// 让安全边际**双向**参与评分（负值扣分），正向档位与阈值**保持不变**
/// （避免动到既有正向样本的档位）。扣分边界沿用 `margin_of_safety.level`
/// 在 −20% / −50% 附近换档的既有分组，避免两处口径打架。
fn value_signal_of(
    mos_pct: Option<f64>,
    f_score: u32,
    moat_score: u32,
    oe_yield: f64,
) -> &'static str {
    let mut score: i32 = 0;
    match mos_pct {
        Some(p) if p > 20.0 => score += 30,
        Some(p) if p > 10.0 => score += 20,
        Some(p) if p > 0.0 => score += 10,
        Some(p) if p > -20.0 => score -= 10,
        Some(p) if p > -50.0 => score -= 25,
        Some(_) => score -= 40,
        None => {},
    }
    score += (f_score.min(9) * 5) as i32;
    score += (moat_score.min(100) / 5) as i32;
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

    // ═══════════════════════════════════════════════════════════════════════
    // 2026-09-21：现金流取值链三态 —— `ttm_fcf` / `basis` / `value_signal`
    //
    // 背景：`vendors/eastmoney.rs` 曾把 `operating_cash_flow` /
    // `capital_expenditure` / `free_cash_flow` 硬编码为 `None` ⇒ `direct_fcf ≡ None`
    // ⇒ DCF 锚点永远走历史均值 fallback、适用性判据 ② 永远无法命中。
    // ═══════════════════════════════════════════════════════════════════════

    /// 带现金流的三段序列构造器
    fn report_cf(date: &str, np: f64, ocf: f64, capex: f64) -> FinancialReport {
        let mut r = report(date, Some(np), None);
        r.operating_cash_flow = Some(ocf);
        r.capital_expenditure = Some(capex);
        r
    }

    /// 300308 中际旭创实测序列（东财，2026-09-21；原始单位为亿元，此处折「元」）
    ///
    /// ```text
    ///   2025FY  OCF 108.96 − capex 27.60 = +81.36
    ///   2026H1  OCF  18.00 − capex 48.02 = −30.02   （半年累计）
    ///   2025H1  OCF  32.18 − capex  9.54 = +22.65
    /// ```
    fn zjxc_financials() -> Vec<FinancialReport> {
        let y = 1.0e8;
        vec![
            report_cf("2026-06-30", 136.51 * y, 18.00 * y, 48.02 * y),
            report_cf("2026-03-31", 57.35 * y, 33.68 * y, 19.29 * y),
            report_cf("2025-12-31", 107.97 * y, 108.96 * y, 27.60 * y),
            report_cf("2025-06-30", 40.00 * y, 32.18 * y, 9.54 * y),
        ]
    }

    /// ① 年报口径本身就是年度值 ⇒ 直接相减，**不做**任何还原/放大。
    #[test]
    fn ttm_fcf_uses_annual_report_directly() {
        let f = vec![report_cf("2025-12-31", 100.0e8, 80.0e8, 30.0e8)];
        let v = ttm_fcf(&f).expect("年报口径应可用");
        assert!((v - 50.0e8).abs() < 1.0, "应直接 OCF−capex = 50 亿，实得 {} 亿", v / 1e8);
    }

    /// ② 中报口径必须还原 TTM —— 这是本函数存在的**唯一理由**。
    ///
    /// 不还原时最新期（2026H1）的 −30.02 亿会被当成年度值，于是一家
    /// 当期 FCF 为正的公司被判成「当期 FCF ≤ 0」⇒ 触发历史均值锚定。
    /// 同一组输入做**反向对照**：还原后为正、不还原为负 ⇒ 二者会走
    /// `compute_dcf` 的不同分支，证明该还原不是装饰。
    #[test]
    fn ttm_fcf_restores_ttm_for_interim_reports() {
        let f = zjxc_financials();
        let v = ttm_fcf(&f).expect("三段齐备应可还原");
        // 81.36 + (−30.02) − 22.65 = +28.69 亿
        assert!((v - 28.69e8).abs() < 0.05e8, "TTM 应为 +28.69 亿，实得 {} 亿", v / 1e8);

        let naive = f[0].operating_cash_flow.unwrap() - f[0].capital_expenditure.unwrap();
        assert!(naive < 0.0, "不还原时最新期累计值为负 —— 这正是要消除的错误面");
        assert!(v > 0.0, "还原后为正 ⇒ 两者会走 compute_dcf 的不同分支（符号相反）");
    }

    /// ③ 缺任一段 ⇒ `None`（**不得**把缺失当 0，也不得退化成单期值）。
    ///
    /// 该形态决定 `basis` 走「现金流量表数据缺失」而非「当期FCF≤0」——
    /// 两者诊断性质完全不同（我方采集缺陷 vs 标的现金流属性）。
    #[test]
    fn ttm_fcf_returns_none_when_any_segment_missing() {
        let y = 1.0e8;
        // 缺「上年同期」
        let f = vec![
            report_cf("2026-06-30", 136.51 * y, 18.00 * y, 48.02 * y),
            report_cf("2025-12-31", 107.97 * y, 108.96 * y, 27.60 * y),
        ];
        assert!(ttm_fcf(&f).is_none(), "缺上年同期应返回 None，不得退化成单期值");

        // 只有 OCF、缺 capital_expenditure ⇒ 无法相减
        let mut only_ocf = report("2026-06-30", Some(136.51 * y), None);
        only_ocf.operating_cash_flow = Some(18.00 * y);
        assert!(ttm_fcf(&[only_ocf]).is_none(), "只有 OCF 无法相减，应 None");
    }

    /// ④ vendor 直供 `free_cash_flow` 时优先采用（不要求 OCF/capex 在场）。
    #[test]
    fn ttm_fcf_falls_back_to_vendor_supplied_free_cash_flow() {
        let mut r = report("2026-06-30", Some(100.0e8), None);
        r.free_cash_flow = Some(12.0e8);
        assert_eq!(ttm_fcf(&[r]).map(|v| v / 1e8), Some(12.0));
    }

    /// 防回归：`ttm_net_profit` 必须与 [`ttm_fcf`] **同样回落**，不得返回 `None`。
    ///
    /// 背景（2026-09-23）：把判据 ②④ 的净利口径收敛到 `ttm_net_profit` 时，
    /// **漏抄了 `ttm_fcf` 的 ③ 回落分支** ⇒ 序列缺「上年年报 / 上年同期」时
    /// 净利侧 `None`、FCF 侧有值 ⇒ `ttm_net_profit(...).filter(...)` 整段短路
    /// ⇒ **判据 ②（FCF 与净利背离）与 ④（亏损 + FCF 收益率过低）一起静默失明**，
    /// `applicability_signals` 恒空 —— 这两条恰是「杠杆畸高 / 亏损且现金流不具
    /// 定价意义」的标的**唯一**的退出通道，失明后它们的 DCF 腿会以全额权重喂给 f5。
    ///
    /// 暴露它的不是评审而是测试：`dcf_inapplicable_for_leveraged_negative_fcf_shape`
    /// 与 `dcf_inapplicable_for_loss_making_with_tiny_fcf_yield` 双双 `signals=[]`。
    /// 本测试把四种形态钉死，防止回落分支再被「顺手删掉」。
    #[test]
    fn ttm_net_profit_falls_back_like_ttm_fcf() {
        // ① 单期（既缺上年年报、也缺上年同期）⇒ 回落最新期值
        let one = vec![report("2026-06-30", Some(-2.2214e8), Some(-0.53))];
        assert_eq!(ttm_net_profit(&one), Some(-2.2214e8), "单期序列必须回落，不得返回 None");

        // ② 有上年年报但缺**上年同期** ⇒ 仍回落（这正是两个真实用例的形态）
        let two = vec![
            report("2026-06-30", Some(30.0e8), Some(0.30)),
            report("2025-12-31", Some(81.36e8), Some(0.80)),
        ];
        assert_eq!(ttm_net_profit(&two), Some(30.0e8), "缺上年同期必须回落");

        // ③ 三期齐全 ⇒ 走 TTM 还原，且**必须与回落值不同**（否则本测试无区分力）
        let three = vec![
            report("2026-06-30", Some(30.0e8), Some(0.30)),
            report("2025-12-31", Some(81.36e8), Some(0.80)),
            report("2025-06-30", Some(22.65e8), Some(0.22)),
        ];
        assert_eq!(
            ttm_net_profit(&three),
            Some(81.36e8 + 30.0e8 - 22.65e8),
            "三期齐全应走 TTM 还原（逐项量纲与实现一致，避免浮点顺序差）"
        );
        assert_ne!(ttm_net_profit(&three), Some(30.0e8), "TTM 值不得等于单期值");

        // ④ 年报口径直接可用（不走 TTM、也不走回落）
        let annual = vec![report("2025-12-31", Some(81.36e8), Some(0.80))];
        assert_eq!(ttm_net_profit(&annual), Some(81.36e8));
    }

    /// ⑤ `compute_dcf` 的锚定口径必须**分三态说真话**。
    ///
    /// 修复前 `direct_fcf == None`（数据缺失）与 `Some(v ≤ 0)`（真为负）共用
    /// 一句「当期FCF≤0（周期底部）」—— 对一家营收 +182.5%、ROE 62.6% 的成长股
    /// 也断言「周期底部」，且该句被 value-investor 原样引用进 `risk_flags`。
    ///
    /// 三态还各自决定 `is_fallback_anchor`：只要锚不是当期真实 FCF 就必须为
    /// `true`（f5 的置信度衰减门依赖它），**缺数态尤其不能漏**。
    #[test]
    fn dcf_basis_tells_three_states_apart() {
        let y = 1.0e8;

        // (a) 真为负：净利为正而真实 FCF ≤ 0 ⇒ 文案说「当期FCF≤0」且判据 ② 命中
        let mut neg = report("2025-12-31", Some(100.0 * y), Some(8.0));
        neg.operating_cash_flow = Some(10.0 * y);
        neg.capital_expenditure = Some(30.0 * y); // FCF = −20 亿
        let a = compute_dcf(&[neg], shares_of(10.0e8), 50.0, None).2.expect("应回传快照");
        assert!(a.is_fallback_anchor, "负 FCF 应走 fallback 锚定");
        assert!(a.basis.contains("当期FCF≤0"), "真为负应说明当期FCF≤0，实得: {}", a.basis);
        assert!(!a.basis.contains("缺失"), "真为负不是数据缺失，实得: {}", a.basis);
        // 2026-09-21：`fcf_data_missing` 必须与 `is_fallback_anchor` **方向相反**才对 ——
        // 本分支两者都是 fallback，但**缺数标记必须为 false**（标的现金流属性 ≠ 我方采集缺陷）。
        // 缺这条反向对照，把字段写成 `= is_fallback_anchor` 也能让 (b) 的正向断言全过。
        assert!(!a.fcf_data_missing, "真为负不是采集缺陷，fcf_data_missing 不得为 true");
        assert!(
            !a.applicable,
            "净利为正而真实 FCF 为负 ⇒ 判据②「符号相反」应命中并判不适用；\
             修复前该列恒为 None 故此判据在生产上从未命中。signals={:?}",
            a.applicability_signals
        );

        // (b) 数据缺失：文案必须说「缺失」，**不得**谎称当期FCF≤0，
        //     且仍须标记为历史代理锚（否则 f5 衰减门在最该衰减的路径上失效）
        let a = compute_dcf(
            &[report("2025-12-31", Some(100.0 * y), Some(8.0))],
            shares_of(10.0e8),
            50.0,
            None,
        )
        .2
        .expect("应回传快照");
        assert!(a.is_fallback_anchor, "缺数同样走 fallback 锚定（锚仍是历史代理）");
        assert!(a.basis.contains("缺失"), "缺数应说明数据缺失，实得: {}", a.basis);
        // 2026-09-21：缺数态**必须**为 true —— 这是 data-quality 上游缺口告警的唯一来源。
        // 与 (a) 合并成一组「同 is_fallback_anchor、异 fcf_data_missing」的对照。
        assert!(a.fcf_data_missing, "现金流量表数据缺失属我方采集缺陷，必须标记");
        assert!(
            !a.basis.contains("当期FCF≤0"),
            "缺数时不得谎称当期FCF≤0（我方采集缺陷 ≠ 标的现金流为负），实得: {}",
            a.basis
        );

        // (c) 有真实正 FCF ⇒ 不走 fallback，basis 为「当期FCF」，判据 ② 不命中
        let mut pos = report("2025-12-31", Some(100.0 * y), Some(8.0));
        pos.operating_cash_flow = Some(80.0 * y);
        pos.capital_expenditure = Some(30.0 * y); // FCF = +50 亿，FCF/净利 = 0.5
        let a = compute_dcf(&[pos], shares_of(10.0e8), 50.0, None).2.expect("应回传快照");
        assert!(!a.is_fallback_anchor, "有真实正 FCF 时不应标记为历史代理锚");
        assert_eq!(a.basis, "当期FCF");
        assert!(!a.fcf_data_missing, "真实正 FCF 时不存在缺口");
        assert!(a.applicable, "FCF/净利 = 0.5 ≥ 0.3 不应判不适用: {:?}", a.applicability_signals);
    }

    /// ⑥ `value_signal` 必须**能表达「高估」**。
    ///
    /// 修复前估值维度只加不减：`mos_pct` 为负落 `_ => {}` 加 0 分，而
    /// F-Score 与护城河恒加 `9×5 + 100/5 = 45`（恰为「合理偏低」档阈值）
    /// ⇒ 无论价格多高都稳落「合理偏低」。
    #[test]
    fn value_signal_can_express_overvaluation() {
        // 300308 实测输入（2026-09-21，样本 ee770189）：
        // 安全边际 −565.4%、F-Score 6、护城河 75、OE 收益率 1.3%
        // 修复前 = 0 + 30 + 15 + 0 = 45 ⇒「合理偏低」，与同 payload 的
        // `margin_of_safety.level`「无（高估风险）」直接矛盾。
        assert_eq!(
            value_signal_of(Some(-565.4), 6, 75, 1.3),
            "高估",
            "深度负安全边际 + 强 F-Score/护城河 不得落「合理偏低」"
        );

        // 反向对照：同样的 F-Score/护城河，仅安全边际转正 ⇒ 档位必须跟着变
        assert_ne!(
            value_signal_of(Some(-565.4), 6, 75, 1.3),
            value_signal_of(Some(25.0), 6, 75, 1.3),
            "档位必须对安全边际敏感 —— 这正是修复前缺失的性质"
        );
        assert_eq!(value_signal_of(Some(25.0), 6, 75, 1.3), "低估");

        // 负向换档边界（与 `margin_of_safety.level` 的既有分组对齐，避免两处口径打架）
        assert_eq!(value_signal_of(Some(-19.9), 6, 75, 1.3), "合理");
        assert_eq!(value_signal_of(Some(-20.1), 6, 75, 1.3), "偏高");
        // 单调性：安全边际越差，档位不得变好
        let worse = value_signal_of(Some(-60.0), 6, 75, 1.3);
        assert_eq!(worse, "高估", "−60% 安全边际应落「高估」，实得 {worse}");

        // mos 缺失时不参与评分（保持原行为）
        assert_eq!(value_signal_of(None, 6, 75, 1.3), "合理偏低");
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
        let (v, a) =
            compute_graham_value(&financials, 7.91, None).expect("归一化后格雷厄姆值应可用");
        assert!(v > 0.0);
        // 2026-09-21: fixture 未提供 revenue_yoy ⇒ 走配置缺省值，
        // 必须与「实测增长率」区分开（两者可信度不同，下游可能分别处置）。
        assert!(a.growth_from_default, "缺 revenue_yoy 应标为缺省值，实际 {a:?}");
        assert!(!a.growth_clamped_upper && !a.growth_clamped_lower, "缺省值不参与封顶判定：{a:?}");
        // 最新为 H1 累计且缺上年同期 → TTM 不可得 → 回退近 5 年报正 EPS 均值 = 0.39
        // g 缺省用 DEFAULT_GROWTH = 0.12（P1-A 后与 DCF 同源），公式内换算为百分数 12
        // ⇒ v = 0.39 × (8.5 + 2×12) × 4.4/4.4 = 0.39 × 32.5 = 12.675
        // （2026-09-22 量纲修复后必须带 ×100 —— 期望值与生产式同形，改一侧必改另一侧）
        let expected = 0.39 * (8.5 + 2.0 * DEFAULT_GROWTH * 100.0);
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
        let (v, _) = compute_graham_value(&financials, 118.94, None).expect("TTM 可还原时应可用");
        // TTM EPS = 2.64 + 1.18 − 1.22 = 2.60
        // 2026-09-22 量纲修复：g 在公式内换算为百分数 ⇒ 乘数 8.5 + 2×12 = 32.5
        let expected = 2.60 * (8.5 + 2.0 * DEFAULT_GROWTH * 100.0);
        assert!((v - expected).abs() < 1e-6, "v={v}, expected={expected}");
        // 回归护栏：绝不能退化成「直接用中报累计 EPS」
        let interim_only = 1.18 * (8.5 + 2.0 * DEFAULT_GROWTH * 100.0);
        assert!(
            (v - interim_only).abs() > 1.0,
            "TTM 未生效，v={v} 仍贴近中报口径值 {interim_only}"
        );
    }

    /// P1-A: 负增长不得被 clamp 抹平为 0（原 clamp(0.0, 0.30) 等于对衰退股默认「零增长」）
    #[test]
    fn graham_does_not_floor_negative_growth_to_zero() {
        let mut financials = vec![report("2025-12-31", Some(5.0e8), Some(1.0))];
        financials[0].revenue_yoy = Some(-50.0); // −50% → g 取下界
        let (v, a) = compute_graham_value(&financials, 10.0, None).expect("年报口径应可用");
        // 2026-09-22 量纲修复：g 在公式内换算为百分数 ⇒ 乘数 8.5 + 2×(−30) = −51.5
        // （修复前误按小数算成 8.5 + 2×(−0.30) = 7.9）。
        // ⚠️ 乘数为负 ⇒ `.max(0.0)` 把它兜为 **0**。这不是「把 g clamp 到 0」——
        //   下方断言锁住 `a.growth` 仍取到下界 −0.30。「估值 0」纯粹来自乘法器为负，
        //   是格雷厄姆原式的固有行为（原式中 g < −4.25% 即出现负乘数），
        //   不是本次修复新引入的缺陷，故**不加**额外兜底去掩盖它。
        //   业务影响：此类标的 `upsidePct` 恒为 −100%。本次影响面所及的 23 个历史
        //   样本中 `g < 0` 者为 **0**（21 个 g=0 完全不受影响、2 个 g=+30%），
        //   故该分支尚未在真实数据上暴露；一旦出现，f5 融合须按
        //   `growth_clamped_lower` 单独降信，而不是让 −100% 直接参与加权。
        let expected = (1.0 * (8.5 + 2.0 * MIN_GROWTH * 100.0)).max(0.0);
        assert!((v - expected).abs() < 1e-6, "v={v}, expected={expected}");
        assert!((v - 0.0).abs() < 1e-12, "负乘数应由 .max(0.0) 兜为 0，实际 {v}");
        assert!(v < 1.0 * 8.5, "负增长估值应低于零增长基准 8.5：{v} vs 8.5");
        // 2026-09-21: 下界方向的偏差与上界相反（内在价值偏高 ⇒ 看多被夸大），
        // 单独标记以便将来分别处置；此处锁住「下界命中必须置位」。
        assert!(a.growth_clamped_lower, "g 封底必须置位，实际 {a:?}");
        assert!(!a.growth_clamped_upper, "下界命中时不得同时置上界：{a:?}");
        assert!((a.growth - MIN_GROWTH).abs() < 1e-12, "g 应取到 {MIN_GROWTH}");
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

    /// 2026-09-21: 增长率顶死上界必须能被下游观测到（否则无法对 graham 腿降信）。
    ///
    /// 300308 实测：`revenue_yoy = 182.5%` ⇒ `g` 被 `MAX_GROWTH` 封顶在 0.30，
    /// 于是 `graham.intrinsicValue = EPS × 68.5`（量纲修复后；修复前误按小数算作
    /// `EPS × 9.1`）⇒ 现价 926.43 下 `upsidePct` 从 **−81.9%** 翻转为 **+36.6%**
    /// （`18.47×68.5 = 1265.20` vs 现价 926.43）。这正是量纲修复影响的 2 个符号
    /// 翻转样本之一（另一个 688114），其余 21 个样本因 `g = 0` 完全不受影响。
    /// 该输出可被纯 PE 恒等式 `(8.5+2g)×4.4/Y ÷ PE − 1` 复现，两边 `EPS` 相消
    /// ⇒ 对高 PE 标的几乎不含公司特定信息。本测试锁住「标记必须置位」。
    #[test]
    fn graham_flags_growth_clamped_upper() {
        let mut financials = vec![report("2025-12-31", Some(2.05e10), Some(18.47))];
        financials[0].revenue_yoy = Some(182.5); // 远超 MAX_GROWTH
        let (v, a) = compute_graham_value(&financials, 926.43, None).expect("应可用");
        assert!(a.growth_clamped_upper, "g 顶死上界必须置位，实际 {a:?}");
        assert!(!a.growth_clamped_lower, "上界命中时不得同时置下界：{a:?}");
        assert!(!a.growth_from_default, "有 revenue_yoy 时不得标为缺省值：{a:?}");
        assert!(
            (a.growth - MAX_GROWTH).abs() < 1e-12,
            "g 应取到上界 {MAX_GROWTH}，实际 {}",
            a.growth
        );
        // 形态护栏：值必须由「EPS × (8.5+2g) × 4.4/Y」给出（g 以**百分数**代入），
        // 不得退化成别的东西 ⇒ 18.47 × (8.5 + 2×30) × 4.4/4.4 = 18.47 × 68.5 = 1265.195
        let expected = 18.47 * (8.5 + 2.0 * MAX_GROWTH * 100.0) * 4.4 / a.bond_yield;
        assert!((v - expected).abs() < 1e-6, "v={v}, expected={expected}");
        // 反向对照：同样 fixture 但营收增速落在区间内 ⇒ 标记必须为 false，
        // 证明上面的 true 不是恒真（不然门禁无区分力）
        financials[0].revenue_yoy = Some(12.0);
        let (_, b) = compute_graham_value(&financials, 926.43, None).expect("应可用");
        assert!(!b.growth_clamped_upper, "区间内不得置位，实际 {b:?}");
        assert!((b.growth - 0.12).abs() < 1e-12, "g 应取实测值，实际 {}", b.growth);
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

        // ④ 判据 ③ **已于 2026-09-23 撤销** —— 终值占比不再进入适用性判定。
        //    本断言保留「③ 不得命中」这一形态（`applicability_signals` 里不该出现
        //    「终值现值」字样），但理由已从「占比低于阈值」变为「该判据不存在了」；
        //    `terminal_value_ratio` 仍是诊断字段，故只断言其取值范围合法。
        assert!(a.perpetual_clamped_by_negative_growth);
        assert!(
            a.terminal_value_ratio > 0.0 && a.terminal_value_ratio < 1.0,
            "终值占比仍须照常产出（诊断字段），实际 {}",
            a.terminal_value_ratio
        );
        assert!(
            !a.applicability_signals.iter().any(|s| s.contains("终值现值")),
            "③ 已撤销，不得再产出该信号: {:?}",
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

    /// 【2026-09-23】判据 ③ 已撤销：**终值占比不得**再影响 `applicable`。
    ///
    /// 本测试是该撤销的**负向锁** —— 防止有人「顺手把 ③ 加回来」：
    /// 同一份财报、同一形态，只改折现率使 `tvr` 跨过 0.7，`applicable` 必须**不变**。
    ///
    /// 为什么不许它当判据（复算见 `output/sci7-tvr-threshold.mjs`）：
    /// `tvr > 0.7` 的临界点落在 `g > 0` 上（`p` 取 2% 或 4% 得出同一临界值）
    /// ⇒ 命中集 ≈ {增长率 ≥ 0}，实际判的是「预测期非衰退」，
    ///   与它声称的「结论由永续假设独裁」无关 ⇒ 零区分力。
    ///
    /// 前身 `dcf_terminal_dominance_signal_tracks_discount_rate` 断言的是相反命题
    /// （折现率 6% 应命中 ③），已随判据一并撤销。
    #[test]
    fn dcf_terminal_ratio_does_not_affect_applicability() {
        let build = || {
            let mut r = report("2025-12-31", Some(10.0e8), Some(0.6));
            r.debt_ratio = Some(30.0);
            r.free_cash_flow = Some(6.0e8); // FCF/净利 = 0.6 ⇒ 不触发判据 ②
            r.revenue_yoy = Some(10.0); // 正增长 ⇒ 永续不被压回 ⇒ tvr 结构性偏高
            vec![r]
        };
        // 低折现率 ⇒ 终值倍数 1/(d−p) 上升 ⇒ tvr 更高。这不是造数据：折现率是面板可配参数。
        let cfg_low =
            ValuationConfig::from_flat_arguments(&serde_json::json!({ "dcf_discount_rate": 6.0 }))
                .expect("扁平参数应可解析");
        let high_tvr =
            compute_dcf(&build(), shares_of(10.0e8), 15.0, Some(&cfg_low)).2.expect("应回传快照");
        let base = compute_dcf(&build(), shares_of(10.0e8), 15.0, None).2.expect("应回传快照");

        // 前置断言：确实造出了「tvr > 0.7」的形态 —— 否则本测试什么都没测到。
        assert!(
            high_tvr.terminal_value_ratio > 0.7,
            "测试前提不成立：低折现率下 tvr 应 > 0.7，实际 {:.1}%",
            high_tvr.terminal_value_ratio * 100.0
        );
        assert!(
            high_tvr.terminal_value_ratio > base.terminal_value_ratio,
            "低折现率应抬高 tvr：{:.1}% vs {:.1}%",
            high_tvr.terminal_value_ratio * 100.0,
            base.terminal_value_ratio * 100.0
        );

        // 断言本体：tvr 跨越 0.7 不得产生任何适用性信号。
        for (label, a) in [("d=6%", &high_tvr), ("d=8.5%", &base)] {
            assert!(
                !a.applicability_signals.iter().any(|s| s.contains("终值现值")),
                "{label}: 终值占比不得再作为适用性判据（③ 已于 2026-09-23 撤销）: {:?}",
                a.applicability_signals
            );
        }
        assert!(base.applicable, "基准配置不应命中任何判据: {:?}", base.applicability_signals);
        assert!(
            high_tvr.applicable,
            "低折现率配置亦不应命中任何判据: {:?}",
            high_tvr.applicability_signals
        );
        // 撤销的是「判据」而不是「数值」：诊断字段仍须产出合法值。
        assert!(high_tvr.terminal_value_ratio > 0.0 && high_tvr.terminal_value_ratio < 1.0);
    }

    /// 终值分母**利差守卫**（2026-09-23 新增，判据 ⑤）：
    /// `p ≥ d − MIN_TERMINAL_SPREAD` 是**用户可配出来**的，而原实现只在算式里
    /// `.max(0.001)` 兜底 ⇒ 终值被放大到 `FCF₅ × (1+p) / 0.001`
    /// （`p = 100%` 时约 2000 倍、正常利差下约 15 倍）且**静默无日志**。
    ///
    /// 两侧都要钉：越界配置必须钳位 + 上报；**正常配置不得被误报**
    /// （否则判据会退化成「让所有标的的 DCF 腿都退出」，那是另一种失效）。
    #[test]
    fn dcf_perpetual_spread_violation_is_reported() {
        // ⚠️ 扁平参数值必须**具名复用**（下方 `CFG_*`）。
        //    本测试首版把 json 里写死的 `8.5` 与断言里的模块常量 `DISCOUNT_RATE` 混用，
        //    而当时两者恰好都等于 `0.085` ⇒ **假绿**。常量一改（0.085 → 0.077），
        //    断言立刻失败并报「应被钳到 0.062，实际 0.07」—— 它其实在拿**配置的 d**
        //    与**模块常量**相减。这正是「手抄值恰好相等」的形态（`MEMORY-RULES.md` K 组）。
        const CFG_D_PCT: f64 = 8.5;
        const CFG_P_PCT: f64 = 9.0;
        let build = || {
            let mut r = report("2025-12-31", Some(10.0e8), Some(0.6));
            r.debt_ratio = Some(30.0);
            r.free_cash_flow = Some(6.0e8); // FCF/净利 = 0.6 ⇒ 不触发判据 ②
            r.revenue_yoy = Some(10.0);
            vec![r]
        };

        // (a) 越界：p = 9% 同时越过两条上限（利差 7.0% / r_f 1.7%）⇒ 命中 ⑤
        let cfg = ValuationConfig::from_flat_arguments(&serde_json::json!({
            "dcf_perpetual_rate": CFG_P_PCT,
            "dcf_discount_rate": CFG_D_PCT,
        }))
        .expect("扁平参数应可解析");
        let (tiers, _, a) = compute_dcf(&build(), shares_of(10.0e8), 15.0, Some(&cfg));
        let a = a.expect("应回传快照");
        assert!(!a.applicable, "永续增长率越界必须判不适用: {:?}", a.applicability_signals);
        let sig = a.applicability_signals.join("；");
        assert!(sig.contains("利差"), "原因应点名利差约束: {sig}");
        assert!(sig.contains("无风险利率"), "原因应点名无风险利率约束: {sig}");
        // 两条上限里更紧的是 r_f（7.0% vs 1.7%）⇒ 生效值必须取 r_f，而不是利差上界。
        // 这条断言正是首版缺失的：首版只比「d − 1.5pp」，一旦 r_f 更紧就会漏判。
        assert_eq!(
            a.perpetual_growth, MAX_PERPETUAL_GROWTH,
            "生效值应取更紧的上限（r_f），实际 {}",
            a.perpetual_growth
        );
        assert!(
            a.perpetual_growth < CFG_D_PCT / 100.0 - MIN_TERMINAL_SPREAD,
            "本用例须落在「r_f 更紧」的区域，否则覆盖不到该分支"
        );
        assert_eq!(
            a.configured_perpetual_growth,
            CFG_P_PCT / 100.0,
            "配置原值必须原样保留以供对账，不得被静默丢弃"
        );
        // 零破坏性：三档数值仍须产出且不乱序（旧模板依赖它们）。
        let (low, mid, high) = tiers.expect("不适用也必须照常产出三档数值（零破坏性）");
        assert!(low <= mid && mid <= high, "档位序不得被新判据破坏");

        // (c) 覆盖**另一分支**：`d` 配到 3% ⇒ 利差上界 1.5pp 比 `r_f` 1.7% 更紧
        //     ⇒ 生效值取利差上界，且文案必须点名「终值分母利差」而不是「无风险利率」。
        //     （缺这条，`spread_is_binding` 的那一侧永远不被执行 —— 上报文案可能撒谎。）
        let cfg2 = ValuationConfig::from_flat_arguments(&serde_json::json!({
            "dcf_perpetual_rate": 5.0,
            "dcf_discount_rate": 3.0,
        }))
        .expect("扁平参数应可解析");
        let a2 = compute_dcf(&build(), shares_of(10.0e8), 15.0, Some(&cfg2)).2.expect("应回传快照");
        assert!(
            (a2.perpetual_growth - (0.03 - MIN_TERMINAL_SPREAD)).abs() < 1e-12,
            "d = 3% 时利差上界 {:.4} 应比 r_f {:.4} 更紧，实际生效值 {}",
            0.03 - MIN_TERMINAL_SPREAD,
            MAX_PERPETUAL_GROWTH,
            a2.perpetual_growth
        );
        assert!(
            a2.applicability_signals.join("；").contains("终值分母利差"),
            "应点名「终值分母利差」为更紧的约束: {:?}",
            a2.applicability_signals
        );

        // (b) 反向：默认配置（p = PERPETUAL_GROWTH、d = DISCOUNT_RATE）**不得**命中 ⑤
        let a = compute_dcf(&build(), shares_of(10.0e8), 15.0, None).2.expect("应回传快照");
        assert!(
            a.applicability_signals.is_empty(),
            "默认配置不应被误报: {:?}",
            a.applicability_signals
        );
        assert_eq!(a.perpetual_growth, PERPETUAL_GROWTH);
    }

    /// 乐观档增长率上界（2026-09-23 修复）：`g × 1.5` **不得**被基准上界吞掉。
    ///
    /// 原实现乐观档与基准档共用 `max_growth` 上界 ⇒ `g ≥ 20%` 时 `g × 1.5` 被砍回
    /// 基准档（`g = 30%` 时实际乘子 = 1.0），而面板文案仍声称「×1.5」
    /// ⇒ **口径与实算不符**。全库 6 条高终值占比样本里 2 条 `g` 正好顶在 30%。
    #[test]
    fn dcf_high_tier_honors_growth_scale_when_base_at_cap() {
        // yoy = 60% ⇒ growth 被 clamp 到 MAX_GROWTH(30%) ⇒ 乐观档应为 30% × 1.5 = 45%
        let mut r = report("2025-12-31", Some(5.0e8), Some(0.3));
        r.revenue_yoy = Some(60.0);
        let (_, _, a) = compute_dcf(&[r], shares_of(10.0e8), 15.0, None);
        let a = a.expect("应回传参数快照");
        assert_eq!(a.growth, MAX_GROWTH, "基准增长率应被 clamp 到 {}%", MAX_GROWTH * 100.0);
        assert!(
            (a.high_growth - MAX_GROWTH * HIGH_GROWTH_SCALE_POS).abs() < 1e-12,
            "乐观档应为 {}% = MAX_GROWTH × {}，实际 {}%（等于基准档 ⇒ 乘子被吞）",
            MAX_GROWTH * HIGH_GROWTH_SCALE_POS * 100.0,
            HIGH_GROWTH_SCALE_POS,
            a.high_growth * 100.0
        );
        assert!(
            a.high_growth > a.growth,
            "上界修复后乐观档必须严格高于基准档：{} vs {}",
            a.high_growth,
            a.growth
        );
    }

    /// 悲观情景的折现率上浮（`RISK_STRESS_SPREAD`）：**只加在悲观档**。
    ///
    /// 依据：折现率是三参数中弹性最大的（300642 终态参数实测 `|E_d| = 1.28` vs
    /// `E_g = 0.70` vs `E_p = 0.17`；`d` 上浮 1pp 使估值变动 1.40 倍 >
    /// `p` 整个 ×0.7~×1.3 档的 1.11 倍）⇒ 把它排除在区间外会使区间宽度**归因错误**。
    /// ⚠️ 弹性随参数变化（`E_p ∝ p/(d−p)²`），引用前须按现行常量复算，见 `sci21`。
    /// 乐观档**不**下调折现率（上界只由经营假设决定，不靠降要求回报灌水）。
    ///
    /// 判据：同一 `g`/`p` 下，「悲观档」必须严格低于「同参数但用基准折现率」的值；
    /// 且基准档取值必须与 `DISCOUNT_RATE` 一致（未被动过）。
    #[test]
    fn dcf_pessimistic_tier_applies_higher_discount_rate() {
        let mut r = report("2025-12-31", Some(10.0e8), Some(0.6));
        r.debt_ratio = Some(30.0);
        r.free_cash_flow = Some(6.0e8);
        r.revenue_yoy = Some(12.0);
        let (tiers, _, a) = compute_dcf(&[r], shares_of(10.0e8), 15.0, None);
        let (low, _, _) = tiers.expect("应有三档");
        let a = a.expect("应回传参数快照");
        assert_eq!(a.discount_rate, DISCOUNT_RATE, "基准档折现率不得被上浮");

        // 手工复算「悲观档若沿用基准折现率」的值，断言真实 low 更低。
        let fcf_ps = 6.0e8 / 10.0e8;
        let no_stress = {
            let mut pv = 0.0;
            let mut cf = fcf_ps;
            for y in 1..=FORECAST_YEARS {
                cf *= 1.0 + a.low_growth;
                pv += cf / (1.0 + DISCOUNT_RATE).powi(y);
            }
            let tv = cf * (1.0 + a.low_perpetual) / (DISCOUNT_RATE - a.low_perpetual);
            pv + tv / (1.0 + DISCOUNT_RATE).powi(FORECAST_YEARS)
        };
        assert!(
            low < no_stress,
            "悲观档必须因要求回报上浮而更低：实际 {low:.4} vs 未上浮 {no_stress:.4}"
        );
        assert!(low > 0.0, "悲观档仍须为正值（上浮不得把估值打成负/零）：{low}");
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
        // 用同一公式把永续按**配置原值**塞回复算，若两侧接近则说明 `perpetual_growth`
        // 根本没被用上。
        //
        // ⚠️ 判据必须是**相对**的：本测试初版写 `> mid_clamped + 1.0`（绝对元），
        //    那是在 `PERPETUAL_GROWTH = 4%` 下标定的。参数降到 1.3% 后同一约束的
        //    绝对影响缩到 **0.86 元**（相对 **14.4%**，语义上毫无削弱）⇒ 断言假红。
        //    绝对容差把「测试的区分力」与「参数的取值」耦死，属**随参数失效**的写法。
        //    改为相对底 5%：当前实测 14.4%，留 2.9 倍余量；若约束真被空转（差值→0）
        //    仍会红。
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
            with_uncapped_perpetual > mid_clamped * 1.05,
            "压回前 mid({with_uncapped_perpetual}) 与压回后({mid_clamped}) 差异不足 5% \
             （相对差 {:.4}）⇒ `perpetual_growth` 未参与实际计算，约束是空转",
            (with_uncapped_perpetual - mid_clamped) / mid_clamped
        );
    }

    // ── 判据 ④（2026-09-21）：亏损公司的符号缺口 ──

    /// **净利为负** + 锚定 FCF 收益率极低 ⇒ 判不适用。
    ///
    /// 回归对象：判据 ② 的两条形态（符号相反 / `FCF/净利 < 0.3`）都写在
    /// `net_profit.filter(|v| *v > 0.0)` 之内 ⇒ **净利为负时整段短路**，
    /// 于是「净利为正但现金流差」的公司被拦下，**真亏损**的反被放行。
    ///
    /// 生产样本（2026-09-21，688114 华大智造）：净利 −2.22 亿、TTM FCF 3.588 亿、
    /// 市值 303.87 亿 ⇒ 收益率 **1.18%**；DCF 三档 16.33 / 24.23 / 38.84 元
    /// 对现价 73.11（`upsidePct = −66.9`），却落 `applicable = true`
    /// 并以 `is_fallback_anchor = false` **全额权重**进 f5。
    ///
    /// 本用例直接复刻该样本的四个量（股本 / 现价 / FCF / 净利）。
    #[test]
    fn dcf_inapplicable_for_loss_making_with_tiny_fcf_yield() {
        let mut latest = report("2026-06-30", Some(-2.2214e8), Some(-0.53));
        latest.debt_ratio = Some(29.0); // 判据 ① 不触发
        latest.revenue_yoy = Some(18.91); // 正增长 ⇒ 判据 ③ 不触发
        latest.free_cash_flow = Some(3.5876e8); // 当期真实正 FCF（判据 ② 短路）
        let a = compute_dcf(&[latest], shares_of(4.15634e8), 73.11, None).2.expect("应回传快照");

        assert!(
            !a.applicable,
            "净利为负 + FCF 收益率 1.18% 应判不适用，实得 signals={:?}",
            a.applicability_signals
        );
        let reason = a.inapplicable_reason.as_deref().expect("不适用必须给出原因");
        assert!(reason.contains("FCF 收益率"), "原因应含 FCF 收益率判据: {reason}");
        // 零破坏性：不适用 ≠ 不可用，三档数值仍须产出。
        assert_eq!(a.basis, "当期FCF", "锚仍应是当期真实 FCF");
        assert!(!a.is_fallback_anchor, "本条不是 fallback 锚定，两个标记正交");
    }

    /// **反向断言**：净利为负但现金流充沛（FCF 收益率 20%）**不得**判不适用。
    ///
    /// 缺这条，实现可以退化成「净利 ≤ 0 即不适用」而全绿 —— 那会误伤
    /// 「一次性减值致亏、经营现金流正常」的公司，也与 V74「周期底部用
    /// 历史正净利归一化锚定」的设计直接冲突。判据必须**两条并列**。
    #[test]
    fn dcf_applicable_for_loss_making_with_healthy_fcf_yield() {
        // 净利 −1 亿（如一次性减值），FCF +20 亿，市值 10 元 × 10 亿股 = 100 亿
        // ⇒ 收益率 20%，远高于阈值 ⇒ DCF 口径成立
        let mut latest = report("2026-06-30", Some(-1.0e8), Some(-0.1));
        latest.debt_ratio = Some(30.0);
        latest.revenue_yoy = Some(8.0);
        latest.free_cash_flow = Some(20.0e8);
        let a = compute_dcf(&[latest], shares_of(10.0e8), 10.0, None).2.expect("应回传快照");

        assert!(
            a.applicable,
            "净利为负但 FCF 收益率 20% 不应判不适用: {:?}",
            a.applicability_signals
        );
        assert_eq!(a.inapplicable_reason, None);
    }
}
