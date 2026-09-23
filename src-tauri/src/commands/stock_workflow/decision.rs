use crate::AppState;
use crate::commands::error::ErrorResponse;
use crate::commands::error_code::stock_workflow as wf_err;
use axagent_agent_macro::agent_command;
use axagent_astock_data::as_of::AsOfContext;
use axagent_entities::stock_analyses;
use axagent_harness::workflow_types::{JsonSchema, Variable, WorkflowEdge, WorkflowNode};
#[cfg(test)]
use axagent_rt_workflow::NodeRuntimeState;
use axagent_rt_workflow::{NodeStatus, Workflow};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::Serialize;
use serde_json::json;
use tauri::State;

/// 单个数据源缺失条目（结构化报告用）
#[derive(Debug, Clone, Serialize)]
pub(crate) struct DataMissingItem {
    pub source: String,
    /// "failed" = 全部 Vendor 降级链失败, "partial" = 成功获取但数据不完整
    pub status: String,
    pub detail: String,
}

/// 聚合预检结果：数据充分/部分缺失/完全不足
#[derive(Debug, Clone)]
pub(crate) enum QualityPrecheckResult {
    /// 数据充分，可以执行
    Pass,
    /// 部分数据缺失但可继续
    Partial(String),
    /// 数据不足，跳过（含结构化缺失清单，供前端展示数据缺失报告）
    Insufficient { summary: String, missing_sources: Vec<DataMissingItem> },
}

/// P1-3: 单数据源预检结果(供多源聚合用)
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SourceCheck {
    /// 该源充分
    Ok,
    /// 该源部分缺失,但可继续
    Partial(String),
    /// 该源完全失败(数据为零或 vendor 报错)
    Failed(String),
}

/// as-of 回放中**具备历史语义**的数据源白名单。
///
/// 判据（源码锚点，非猜测）：`crates/astock-data/src/lib.rs` 只为这两个维度实现了
/// as-of 支持 —— `:953` 把 K 线截断到截止日、`:1719-1739` 用截断后的 K 线**合成** quote。
/// 其余维度在 vendor 侧**没有历史接口**：请求发出的始终是「当前」数据，as-of 过滤层
/// 只能把它们丢弃（实测日志逐源可见：`新闻全部晚于截止日` / `财务数据返回空`）
/// ⇒ **在回放中必然为空**。
///
/// ⇒ 这类失败不是「数据质量差」，而是「**该维度在时间旅行里不存在**」。用今天的完整性
/// 口径去要求历史回放，等于要求数据商提供它从未提供过的历史新闻 —— 2026-09-23 实测：
/// 53 条历史分析**全部**因此被 `Insufficient` 拦住，DAG 一条都没跑（4/4 `failed`）。
const AS_OF_HISTORICAL_SOURCES: &[&str] = &["quote", "klines"];

/// P1-3: 聚合 5 个核心数据源的预检结果, 取最差等级
///
/// `as_of_mode`：当前是否处于 as-of 回放。
/// 为 `true` 时，**无历史语义的维度**（见 `AS_OF_HISTORICAL_SOURCES`）的 `Failed`
/// 降级为 `Partial` —— 仍**留痕**（进 summary，前端数据缺失报告与实盘回放可信度
/// 评估都靠它）但**不阻断** DAG。
/// `quote` / `klines` 保持原判定：它们失败意味着 as-of 引擎自己拿不到数据，
/// 那是真故障，必须继续阻断（否则回放会拿空气当行情）。
pub(crate) fn aggregate_precheck(
    sources: Vec<(&str, SourceCheck)>,
    as_of_mode: bool,
) -> QualityPrecheckResult {
    let mut partial_msgs: Vec<String> = Vec::new();
    let mut missing_sources: Vec<DataMissingItem> = Vec::new();
    for (name, c) in sources {
        match c {
            SourceCheck::Ok => {},
            SourceCheck::Partial(reason) => partial_msgs.push(format!("{name}: {reason}")),
            SourceCheck::Failed(reason) => {
                if as_of_mode && !AS_OF_HISTORICAL_SOURCES.contains(&name) {
                    // as-of：该维度无历史接口 ⇒ 空是**结构性**的，不是质量缺陷。
                    // 降级为 Partial（不阻断），但把原文与原因一起留在 summary 里，
                    // 便于事后判断「这条回放结论的哪些输入是缺的」。
                    partial_msgs.push(format!(
                        "{name}: {reason}（as-of 回放：该维度无历史接口，空属结构性，不阻断）"
                    ));
                } else {
                    missing_sources.push(DataMissingItem {
                        source: name.to_string(),
                        status: "failed".into(),
                        detail: reason,
                    });
                }
            },
        }
    }
    if !missing_sources.is_empty() {
        let summary = missing_sources
            .iter()
            .map(|item| format!("{}: {}", item.source, item.detail))
            .collect::<Vec<_>>()
            .join("; ");
        QualityPrecheckResult::Insufficient { summary, missing_sources }
    } else if !partial_msgs.is_empty() {
        QualityPrecheckResult::Partial(partial_msgs.join("; "))
    } else {
        QualityPrecheckResult::Pass
    }
}

/// 在启动 DAG 前执行快速数据质量检查。
///
/// P1-3 修复: 扩展预检覆盖 5 个核心数据源(quote / financials / klines / news /
/// money_flow),任一完全失败则整体 Insufficient;部分缺失则 Partial。as-of 模式下
/// 所有 vendor 调用走 as-of scope, 预检结果反映"截至 as_of_date 的数据是否够用"。
///
/// API 调用成本: 5 次 vs 原 2 次, 仍远低于 15~20 次 LLM 调用。
///
/// P1-1 修复(2026-08-09): 接入 astock-data/validation.rs 的字段级校验
/// (validate_quote/validate_financials/validate_klines/validate_news_batch)，
/// 将"只查非空/行数"升级为 OHLC 有效性、high<low、eps/roe 值域等字段检查。
/// P2-3 修复(2026-08-09): 新增跨源一致性校验——
///   quote.price vs 最新 K 线 close(偏差>1%)、quote.pe vs 财报 EPS 隐含 PE(>20%)、
///   K 线最新日期未来/滞后检查。命中即 Partial 告警,不阻断。
pub(crate) async fn data_quality_precheck(
    client: &axagent_astock_data::AStockClient,
    stock_code: &str,
    quote: &axagent_astock_data::StockQuote,
) -> QualityPrecheckResult {
    use axagent_astock_data::validation::{
        validate_financials, validate_klines, validate_news_batch, validate_quote,
    };

    // 1. quote — 字段级校验（code/price/name 缺失 → Failed；change_pct/pre_close 异常 → Partial）
    let quote_check = {
        let vr = validate_quote(quote);
        if !vr.missing.is_empty() {
            SourceCheck::Failed(vr.missing.join("; "))
        } else if !vr.warnings.is_empty() {
            SourceCheck::Partial(vr.warnings.join("; "))
        } else {
            SourceCheck::Ok
        }
    };

    // 2. financials — 营收/利润存在性 + 字段值域校验（eps/roe 异常）+ PE 交叉校验
    let fin_check = match client.get_financials(stock_code).await {
        Ok(financials) => {
            let has_revenue = financials.iter().any(|f| f.revenue.unwrap_or(0.0) > 0.0);
            let has_profit = financials.iter().any(|f| f.net_profit.unwrap_or(0.0) > 0.0);
            let vr = validate_financials(&financials);
            if !has_revenue && !has_profit {
                // 空财报/营收利润缺失：保持原 Partial 语义（可继续但基本面受限），不升级 Failed
                SourceCheck::Partial("营收/利润缺失".into())
            } else if !vr.missing.is_empty() {
                SourceCheck::Failed(vr.missing.join("; "))
            } else if !vr.warnings.is_empty() {
                SourceCheck::Partial(vr.warnings.join("; "))
            } else {
                // P2-3: quote.pe vs 最新财报 EPS 隐含 PE 交叉校验
                let mut cross: Vec<String> = Vec::new();
                if let (Some(pe), Some(eps)) = (quote.pe, financials.first().and_then(|f| f.eps)) {
                    if pe > 0.0 && eps > 0.0 && quote.price > 0.0 {
                        let implied_pe = quote.price / eps;
                        let dev = (implied_pe - pe).abs() / pe;
                        if dev > 0.2 {
                            cross.push(format!(
                                "行情 PE {pe} 与财报 EPS 隐含 PE {implied_pe:.1} 偏差 {:.0}%",
                                dev * 100.0
                            ));
                        }
                    }
                }
                if cross.is_empty() {
                    SourceCheck::Ok
                } else {
                    SourceCheck::Partial(cross.join("; "))
                }
            }
        },
        Err(e) => SourceCheck::Failed(format!("全部数据源获取失败: {e}")),
    };

    // V38 修复: K 线至少需要 60 日才能计算 MA(20)+MACD(26) 等关键技术指标。
    // 不足 60 日但 ≥30 日时仅降级为 Partial（可继续但技术分析受限）。
    // P1-1(2026-08-09): 行数足够时追加 validate_klines 字段校验（OHLC/high<low 等）。
    // P2-3(2026-08-09): 追加 quote.price vs 最新 close 偏差、K 线日期时效交叉校验。
    let kline_check = match client.get_klines(stock_code, "daily", 500).await {
        Ok(klines) if klines.len() >= 60 => {
            let vr = validate_klines(&klines);
            if !vr.missing.is_empty() {
                SourceCheck::Partial(format!("K 线字段异常: {}", vr.missing.join("; ")))
            } else {
                let mut cross: Vec<String> = Vec::new();
                if let Some(last) = klines.last() {
                    if last.close > 0.0 && quote.price > 0.0 {
                        let dev = (quote.price - last.close).abs() / last.close;
                        if dev > 0.01 {
                            cross.push(format!(
                                "最新 K 线收盘 {} 与行情价 {} 偏差 {:.1}%",
                                last.close,
                                quote.price,
                                dev * 100.0
                            ));
                        }
                    }
                    // 日期时效: 未来日期(>当前/回放日)或滞后 >10 自然日 → 告警
                    let asof = axagent_astock_data::as_of::current_date_or_now();
                    if let (Ok(d1), Ok(d2)) = (
                        chrono::NaiveDate::parse_from_str(&last.date, "%Y-%m-%d"),
                        chrono::NaiveDate::parse_from_str(&asof, "%Y-%m-%d"),
                    ) {
                        let days = (d2 - d1).num_days();
                        if days < 0 {
                            cross.push(format!(
                                "K 线最新日期 {} 晚于当前/回放日 {}",
                                last.date, asof
                            ));
                        } else if days > 10 {
                            cross.push(format!(
                                "K 线数据滞后 {days} 天(最新 {} vs 当前/回放日 {asof})",
                                last.date
                            ));
                        }
                    }
                }
                if cross.is_empty() {
                    SourceCheck::Ok
                } else {
                    SourceCheck::Partial(cross.join("; "))
                }
            }
        },
        Ok(klines) if klines.len() >= 30 => {
            SourceCheck::Partial(format!("仅 {} 行, 技术分析受限", klines.len()))
        },
        Ok(klines) if !klines.is_empty() => {
            SourceCheck::Partial(format!("仅 {} 行, 严重不足", klines.len()))
        },
        Ok(_) => SourceCheck::Failed("K 线为空".into()),
        Err(e) => SourceCheck::Failed(format!("全部数据源获取失败: {e}")),
    };

    // P1-3 新增: 4. news (取最近 10 条)
    // P1-1(2026-08-09): 非空时追加 title/url/publish_time 字段校验
    let news_check = match client.get_news(stock_code, 10).await {
        Ok(news) if !news.is_empty() => {
            let vr = validate_news_batch(&news);
            if !vr.missing.is_empty() {
                SourceCheck::Partial(format!("新闻字段异常: {}", vr.missing.join("; ")))
            } else {
                SourceCheck::Ok
            }
        },
        Ok(_) => SourceCheck::Partial("无新闻数据".into()),
        Err(e) => SourceCheck::Failed(format!("全部数据源获取失败: {e}")),
    };

    // P1-3 新增: 5. money_flow
    let money_flow_check = match client.get_money_flow(stock_code).await {
        Ok(Some(_)) => SourceCheck::Ok,
        Ok(None) => SourceCheck::Partial("无资金流数据".into()),
        Err(e) => SourceCheck::Failed(format!("全部数据源获取失败: {e}")),
    };

    // P2: 补充数据源检查 — 覆盖 catalyst-analyst / sector-analyst 的依赖
    let announcements_check = match client.get_announcements(stock_code).await {
        Ok(anns) if !anns.is_empty() => SourceCheck::Ok,
        Ok(_) => SourceCheck::Partial("无公告数据".into()),
        Err(e) => SourceCheck::Failed(format!("全部数据源获取失败: {e}")),
    };
    let concept_check = match client.get_concept_blocks(stock_code).await {
        Ok(Some(blocks)) if !blocks.concepts.is_empty() => SourceCheck::Ok,
        Ok(_) => SourceCheck::Partial("无概念板块数据".into()),
        Err(e) => SourceCheck::Failed(format!("概念板块数据源全部获取失败: {e}")),
    };

    // V40 修复: 补充对核心分析师依赖的数据源预检（不阻塞分析，仅标记 Partial）
    // a-sector / a-catalyst 依赖 sector_info；a-lockup 依赖 lockup_schedule
    let sector_check = match client.get_sector_info(stock_code).await {
        Ok(Some(_)) => SourceCheck::Ok,
        Ok(None) => SourceCheck::Partial("无行业板块数据".into()),
        Err(e) => SourceCheck::Failed(format!("行业板块数据源全部获取失败: {e}")),
    };
    let lockup_check = match client.get_lockup_schedule(stock_code).await {
        Ok(items) if !items.is_empty() => SourceCheck::Ok,
        Ok(_) => SourceCheck::Partial("无限售解禁数据".into()),
        Err(e) => SourceCheck::Failed(format!("限售解禁数据源全部获取失败: {e}")),
    };
    // PACE 集成: 补充 dragon_tiger 预检（筹码面 f10 增强所需的机构席位数据）
    let dragon_tiger_check = match client.get_dragon_tiger(stock_code).await {
        Ok(entries) if !entries.is_empty() => SourceCheck::Ok,
        Ok(_) => SourceCheck::Partial("无龙虎榜数据".into()),
        Err(e) => SourceCheck::Failed(format!("龙虎榜数据源全部获取失败: {e}")),
    };

    // 全部数据源统一由 aggregate_precheck 判定：
    // - 任一数据源 Failed（所有 Vendor 降级链均失败）→ 整体 Insufficient，阻断工作流
    // - 全部通过但存在 Partial（成功获取但某维度天然空）→ 整体 Partial，继续但标记警告
    // - 全部通过且无 Partial → Pass
    // ⚠ as-of 回放例外：无历史语义的维度（news/financials/…）必然为空，
    // 由第二参 `as_of_mode` 触发降级为 Partial —— 见 `AS_OF_HISTORICAL_SOURCES` 注释。
    aggregate_precheck(
        vec![
            ("quote", quote_check),
            ("financials", fin_check),
            ("klines", kline_check),
            ("news", news_check),
            ("money_flow", money_flow_check),
            ("announcements", announcements_check),
            ("concept_blocks", concept_check),
            ("sector_info", sector_check),
            ("lockup_schedule", lockup_check),
            ("dragon_tiger", dragon_tiger_check),
        ],
        axagent_astock_data::as_of::current_as_of().is_some(),
    )
}

pub(crate) struct LoadedTemplate {
    pub nodes: Vec<WorkflowNode>,
    pub edges: Vec<WorkflowEdge>,
    pub input_schema: Option<JsonSchema>,
    pub output_schema: Option<JsonSchema>,
    pub variables: Option<Vec<Variable>>,
    /// 模板行自带的版本号（`workflow_templates.version`）。
    ///
    /// 用途：决策落库时写进 `stock_analyses.template_version`，使**历史决策可离线复算**。
    /// 版本号是公式版本的合法代理 —— 脚本经 `include_str!` 嵌入模板，改公式必升
    /// `TEMPLATE_VERSION`。少了它，用现行公式复算旧样本会系统性偏高 4.5pt
    /// （2026-09-18 实测，见 `portfolio-mgr.rhai` 复算注释）。
    pub version: i32,
    /// 模板声明的生命周期钩子（pre_exec/post_exec）。
    /// 必须随 create_workflow_with_hooks 传入引擎，否则 pre_exec 的
    /// stock-analysis-enhance 钩子不执行 → market_regime 等业务变量
    /// 永不注入 → a-fundamentals 等节点 VARIABLE_NOT_FOUND（2026-09-08 实证）。
    pub hooks_config: Option<axagent_harness::WorkflowHooksConfig>,
}

/// 从模板变量解析 vendor_* 布尔开关，注入到 astock_client 的启用状态过滤器。
/// 未启用的 vendor 会在 find_vendor 中被跳过，避免无效调用和超时重试。
/// 应在 load_and_inject_template 之后、run_workflow 之前调用。
pub(crate) fn inject_vendor_state(
    astock_client: &axagent_astock_data::AStockClient,
    variables: Option<&Vec<Variable>>,
) {
    if let Some(vars) = variables {
        let template_vars: Vec<(String, serde_json::Value)> =
            vars.iter().map(|v| (v.name.clone(), v.value.clone())).collect();
        let enabled_set =
            axagent_analysis_engine::recommender::pool::load_enabled_vendors_from_template(
                &template_vars,
            );
        tracing::info!("[stock_workflow] vendor 启用状态已注入: {:?}", enabled_set);
        astock_client.set_enabled_vendors(Some(enabled_set));
    }
}

#[cfg(test)]
mod precheck_tests {
    use super::*;

    // P1-3: aggregate_precheck 取最差等级
    #[test]
    fn aggregate_all_ok_returns_pass() {
        let r = aggregate_precheck(
            vec![
                ("quote", SourceCheck::Ok),
                ("financials", SourceCheck::Ok),
                ("klines", SourceCheck::Ok),
            ],
            false,
        );
        assert!(matches!(r, QualityPrecheckResult::Pass));
    }

    #[test]
    fn aggregate_one_partial_returns_partial_with_joined_message() {
        let r = aggregate_precheck(
            vec![
                ("quote", SourceCheck::Ok),
                ("financials", SourceCheck::Partial("营收缺失".into())),
                ("klines", SourceCheck::Ok),
            ],
            false,
        );
        match r {
            QualityPrecheckResult::Partial(msg) => {
                assert!(msg.contains("financials"), "partial msg 应含 source 名: {msg}");
                assert!(msg.contains("营收缺失"));
            },
            _ => panic!("expected Partial"),
        }
    }

    #[test]
    fn aggregate_any_failure_returns_insufficient() {
        let r = aggregate_precheck(
            vec![
                ("quote", SourceCheck::Ok),
                ("klines", SourceCheck::Failed("K 线获取失败".into())),
            ],
            false,
        );
        match r {
            QualityPrecheckResult::Insufficient { summary, .. } => {
                assert!(
                    summary.contains("klines"),
                    "insufficient summary 应含 source 名: {summary}"
                );
                assert!(summary.contains("K 线获取失败"));
            },
            _ => panic!("expected Insufficient"),
        }
    }

    #[test]
    fn aggregate_failure_beats_partial() {
        // 5 源: 2 partial + 1 failed → overall Insufficient
        let r = aggregate_precheck(
            vec![
                ("quote", SourceCheck::Ok),
                ("financials", SourceCheck::Partial("缺".into())),
                ("klines", SourceCheck::Failed("空了".into())),
                ("news", SourceCheck::Partial("无".into())),
                ("money_flow", SourceCheck::Ok),
            ],
            false,
        );
        assert!(matches!(r, QualityPrecheckResult::Insufficient { .. }));
    }

    // ── as-of 分源判定（2026-09-23）──
    // 锁住那次阻断：53 条历史分析曾因 news 在回放中必然为空而**全部跑不动**（4/4 failed）。

    #[test]
    fn aggregate_as_of_ignores_sources_without_history_semantics() {
        let r = aggregate_precheck(
            vec![
                ("quote", SourceCheck::Ok),
                ("klines", SourceCheck::Ok),
                ("news", SourceCheck::Failed("新闻全部晚于截止日".into())),
            ],
            true,
        );
        match r {
            QualityPrecheckResult::Partial(msg) => {
                assert!(msg.contains("news"), "降级后仍须留痕: {msg}");
                assert!(msg.contains("无历史接口"), "应标明降级原因: {msg}");
            },
            other => panic!("as-of 不得因 news 阻断，期望 Partial，实得 {other:?}"),
        }
    }

    #[test]
    fn aggregate_as_of_still_blocks_when_historical_source_fails() {
        // 反向断言：as-of 下 quote/klines 失败仍是**真故障**，必须阻断
        // （否则回放会拿空气当行情，上面那条放宽就成了 fail-open）。
        let r = aggregate_precheck(
            vec![
                ("quote", SourceCheck::Ok),
                ("klines", SourceCheck::Failed("K 线为空".into())),
                ("news", SourceCheck::Failed("新闻全部晚于截止日".into())),
            ],
            true,
        );
        match r {
            QualityPrecheckResult::Insufficient { summary, .. } => {
                assert!(summary.contains("klines"), "summary 应含 klines: {summary}");
                assert!(!summary.contains("news"), "news 已降级，不应进阻断清单: {summary}");
            },
            other => panic!("klines 失败必须阻断，期望 Insufficient，实得 {other:?}"),
        }
    }

    #[test]
    fn aggregate_live_mode_keeps_news_as_blocker() {
        // live 反向锁：同样的 news Failed，**实盘必须仍然阻断** ——
        // 放宽只对 as_of 生效，绝不能渗到实盘路径。
        let r = aggregate_precheck(
            vec![("news", SourceCheck::Failed("全部数据源获取失败".into()))],
            false,
        );
        assert!(matches!(r, QualityPrecheckResult::Insufficient { .. }));
    }
}

pub(crate) async fn load_and_inject_template(
    db: &sea_orm::DatabaseConnection,
    stock_code: &str,
    _stock_name: &str,
    template_id: &str,
) -> Result<LoadedTemplate, String> {
    use axagent_entities::workflow_template;

    // 运行前确保模板为最新版（幂等：版本已是最新则跳过）。
    // 与 serenity.rs 同款兜底：启动链路无 stock_analysis_setup 种子调用，
    // DB 版本可能停留在旧版（实证：代码 v13-v15 期间 DB 停在 v12，V59 修复
    // 静默失效三天）。每次运行前查一次版本，旧版自动升级。
    crate::commands::stock_analysis_setup::ensure_stock_analysis_experts_seeded(db)
        .await
        .map_err(|e| format!("模板版本兜底重种子化失败: {e}"))?;

    let template = workflow_template::Entity::find_by_id(template_id)
        .one(db)
        .await
        .map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("查询工作流模板失败: {e}"))
        })?
        .ok_or(format!("工作流模板 {template_id} 未种子化，请重启应用"))?;

    let mut nodes: Vec<WorkflowNode> = serde_json::from_str(&template.nodes).map_err(|e| {
        ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("解析模板节点失败: {e}"))
    })?;
    let edges: Vec<WorkflowEdge> = serde_json::from_str(&template.edges).map_err(|e| {
        ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("解析模板边失败: {e}"))
    })?;

    if nodes.is_empty() {
        tracing::warn!("[stock_workflow] 模板节点为空，自动重新种子化");
        crate::commands::stock_analysis_setup::ensure_stock_analysis_experts_seeded(db).await?;
        let template = workflow_template::Entity::find_by_id("stock-analysis")
            .one(db)
            .await
            .map_err(|e| {
                ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("重查模板失败: {e}"))
            })?
            .ok_or("模板种子化后仍不存在")?;
        nodes = serde_json::from_str(&template.nodes).map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("解析模板节点失败: {e}"))
        })?;
    }

    for node in &mut nodes {
        if let WorkflowNode::Trigger(tn) = node {
            if let Some(sc) = tn.config.config.get_mut("stock_code") {
                *sc = serde_json::Value::String(stock_code.to_string());
            }
        }
    }

    // stock_code/stock_name 已通过 AgentNodeConfig.input_mapping 自动注入到每个 Agent 节点的 system_prompt，
    // 不再需要手动遍历追加（参见 stock_analysis_setup.rs 中 agent() 宏的 input_mapping 配置）。

    let input_schema: Option<JsonSchema> =
        template.input_schema.as_ref().and_then(|s| serde_json::from_str(s).ok());
    let output_schema: Option<JsonSchema> =
        template.output_schema.as_ref().and_then(|s| serde_json::from_str(s).ok());
    // ⚠ 解析失败**必须留痕**，不可用 `.ok()` 静默降级：
    // `None` 的后果是下游所有 `input_mapping`（`stock_code` 属 input_params 兜底除外）
    // 解析不到并回退模块常量 —— 「面板上改了、实际没生效」这类缺陷在行为上几乎不可察觉。
    // D8 取证已确认该降级通道是估值参数分叉的成因之一（详见
    // `AUDIT-300642-run-variance-2026-09-22.md` §13）。
    let variables: Option<Vec<Variable>> =
        template.variables.as_ref().and_then(|v| match serde_json::from_str(v) {
            Ok(vars) => Some(vars),
            Err(e) => {
                tracing::warn!(
                    "[stock_workflow] 模板 {template_id} 的 variables 解析失败，按无变量处理\
                     （下游 input_mapping 将回退模块常量）: {e}"
                );
                None
            },
        });
    // 与 rt-workflow parse_hooks_config 同语义：NULL 合法 → None；解析失败降级 None
    let hooks_config: Option<axagent_harness::WorkflowHooksConfig> =
        template.hooks_config.as_ref().and_then(|s| match serde_json::from_str(s) {
            Ok(cfg) => Some(cfg),
            Err(e) => {
                tracing::warn!(
                    "[stock_workflow] 模板 {template_id} hooks_config 解析失败，按无钩子处理: {e}"
                );
                None
            },
        });

    Ok(LoadedTemplate {
        nodes,
        edges,
        input_schema,
        output_schema,
        variables,
        // 决策落库时写进 stock_analyses.template_version，供离线复算判定公式版本
        version: template.version,
        hooks_config,
    })
}

/// 工作流结果 → blackboard_snapshot — 现已委托给 axagent-stock-analysis::blackboard 模块
/// 此处保留占位以便未来重新内联
#[allow(clippy::type_complexity)]
pub(crate) fn extract_decision_fields(
    decision_json: &Option<String>,
) -> (Option<String>, Option<f64>, Option<String>, Option<String>, Option<u32>) {
    let raw = match decision_json {
        Some(s) if !s.is_empty() => s,
        _ => return (None, None, None, None, None),
    };
    let parsed: serde_json::Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(_) => return (None, None, None, None, None),
    };
    let action = parsed.get("action").and_then(|v| v.as_str()).map(|s| s.to_string());
    let position_pct =
        parsed.get("positionPct").or_else(|| parsed.get("position_pct")).and_then(|v| v.as_f64());
    let reasoning = parsed.get("reasoning").and_then(|v| v.as_str()).map(|s| s.to_string());
    let time_horizon = parsed
        .get("timeHorizon")
        .or_else(|| parsed.get("time_horizon"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let expected_holding_days = parsed
        .get("expectedHoldingDays")
        .or_else(|| parsed.get("expected_holding_days"))
        .and_then(|v| {
            if v.is_number() {
                v.as_u64().map(|n| n as u32)
            } else {
                None
            }
        });
    (action, position_pct, reasoning, time_horizon, expected_holding_days)
}

/// 落库前的 action 归一化 —— `stock_analyses.decision_action` 的**唯一写入口**。
///
/// 为什么必须唯一：`decision_action` 有两个落库点（`stock_workflow/core.rs` 的
/// 工作区路径、`stock_workflow/hooks.rs` 的对话直执行路径）。任一处不做归一化，
/// DB 值域就会重新漂移成中/英/未识别三种形态共存 —— 这正是本次 P1-4 要消灭的形态
/// （同名语义散成多套值域 ⇒ 必有一处 fail-open）。
///
/// 规则：
/// - 可识别值 → 规范中文标签（与 `portfolio-mgr.rhai` 的 6 档一致）
/// - 未识别值 → 「不确定」+ WARN（**不原样入库**，也不伪装成「观望」）
/// - `None`（决策缺失）→ `None`，由消费端按缺失处理
pub(crate) fn normalize_action_for_storage(raw: Option<&str>) -> Option<String> {
    use axagent_analysis_engine::decision_action::{ActionKind, normalize_action};
    raw.map(|a| match normalize_action(a) {
        Some(kind) => kind.as_storage_cn().to_string(),
        None => {
            tracing::warn!(
                raw_action = %a,
                "[decision] 落库前遇到未识别 action 值域，归入「不确定」"
            );
            ActionKind::Uncertain.as_storage_cn().to_string()
        },
    })
}

/// 从决策 JSON 中提取与 action **正交**的持仓状态轴（`P1-2`）。
///
/// 缺失 / 空串 → `None`：NULL 的语义是「采集时点无此信息」，消费端应按
/// `decisionPositionPct` 自行派生展示，**不得**读成 `EMPTY`。
pub(crate) fn extract_position_state(decision_json: &Option<String>) -> Option<String> {
    let raw = decision_json.as_deref().filter(|s| !s.is_empty())?;
    let parsed: serde_json::Value = serde_json::from_str(raw).ok()?;
    parsed
        .get("positionState")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 从决策 JSON 中提取**四周期价位映射**（阶段1，PROPOSAL-stock-decision-four-horizon.md）。
///
/// 值形态：`{"ultra_short":{...},"short":{...},"mid":{...},"long":{...}}`，每组含
/// `stopLossPct`/`takeProfitPct`/`expectedHoldingDays`/`targetPrice`/`stopLoss`。
/// 该值由 `portfolio-mgr.rhai` 产出并嵌套在 `decision_json` 全文里，本函数把它
/// 抽成独立 JSON 子串落 `stock_analyses.horizon_price_map`（避免落库后全文反键）。
///
/// **缺省语义**：键缺失 / 空串 / 解析失败 → `None`（NULL = 采集时点无此信息，
/// 或该决策产生于 stage1 字段引入前）。消费端**不得**读成空映射，应按主档位
/// `decision_json` 的 `targetPrice`/`stopLoss` 回退。这与 `extract_position_state`
/// 的 NULL 约定一致。
pub(crate) fn extract_horizon_price_map(decision_json: &Option<String>) -> Option<String> {
    let raw = decision_json.as_deref().filter(|s| !s.is_empty())?;
    let parsed: serde_json::Value = serde_json::from_str(raw).ok()?;
    let obj = parsed.as_object()?;
    let val = obj.get("horizonPriceMap").or_else(|| obj.get("horizon_price_map"))?;
    if val.is_null() {
        return None;
    }
    serde_json::to_string(val).ok()
}

/// 从决策 JSON 中提取**四周期独立决策**（阶段2，PROPOSAL-stock-decision-four-horizon.md）。
///
/// 值形态：`{"ultra_short":{...},"short":{...},"mid":{...},"long":{...}}`，每组含
/// `action`/`verdict`/`positionPct`/`confidence`/`stopLossPct`/`takeProfitPct`/
/// `expectedHoldingDays`，仅 `ultra_short` 额外含 `confLowerBound`（方案B，固定 0.35）。
/// 该值由 `portfolio-mgr.rhai` 产出并嵌套在 `decision_json` 全文里，本函数把它抽成
/// 独立 JSON 子串落 `stock_analyses.horizon_decisions`（与 `extract_horizon_price_map`
/// 同 text 形态、同理由：避免落库后全文反键）。
///
/// **缺省语义**：键缺失 / 空串 / 解析失败 → `None`（NULL = 采集时点无此信息，或该决策
/// 产生于 stage2 字段引入前）。消费端**不得**读成空映射，应按主档位 `decision_action`
/// 回退。这与 `extract_position_state` 的 NULL 约定一致。
pub(crate) fn extract_decisions_by_horizon(decision_json: &Option<String>) -> Option<String> {
    let raw = decision_json.as_deref().filter(|s| !s.is_empty())?;
    let parsed: serde_json::Value = serde_json::from_str(raw).ok()?;
    let obj = parsed.as_object()?;
    let val = obj.get("decisionsByHorizon").or_else(|| obj.get("decisions_by_horizon"))?;
    if val.is_null() {
        return None;
    }
    serde_json::to_string(val).ok()
}

/// 从 Workflow 结果中提取 portfolio-mgr 节点的决策 JSON 字符串。
///
/// 优先取 `results["portfolio-mgr"]["result"]`（CodeNode 包装内 Rhai 脚本的
/// 实际输出，例如 `{ action, positionPct, confidence, ... }`），回退到
/// `results["portfolio-mgr"]` 本身（兼容非 CodeNode 包装的旧版 portfolio-mgr），
/// 最后回退到 workflow 顶层 `output`（兼容无 portfolio-mgr 节点的工作流）。
///
/// 修复"决策信息缺失"误报：之前直接用 `wf.output` 写入 decisionJson，
/// 但 stock-analysis 工作流配置了 output_schema（且未用 $source 标记字段
/// 来源节点），导致 `filter_by_schema` 退化为整个 results map。前端
/// normalizeDecision 拿到 results map 后会判定为"全零空壳"返回 null，
/// store.decision 保持空 → DecisionBanner 显示"决策信息缺失"误报。
/// 将 rt-workflow 节点错误里的错误码资源键转为可读中文。
///
/// 节点错误的统一格式是 "{CODE}: {detail}"（见 rt-workflow
/// node_executor_trait::NodeError 的 Display），其中 CODE 是前端 i18n
/// 资源键（node_executor_trait::error_code 常量），直接透传给用户会显示成
/// 资源键而非可读文案。此处按已知码翻译前缀，未匹配时原样返回
/// （Rhai 执行错误等本身已是可读中文）。错误码与 rt-workflow 的
/// error_code 常量逐一对齐（引用常量而非字面量），上游增删码时
/// cargo 会在此处报错提示同步。
fn humanize_node_error(error_msg: &str) -> String {
    use axagent_rt_workflow::work_engine::node_executor_trait::error_code as code;

    /// (错误码常量, 可读中文文案) 对照表
    const CODE_LABELS: &[(&str, &str)] = &[
        (code::PROVIDER_QUERY_FAILED, "数据供应商查询失败"),
        (code::NO_AVAILABLE_PROVIDER, "无可用数据供应商"),
        (code::API_KEY_DECRYPT_FAILED, "API 密钥解密失败"),
        (code::UNSUPPORTED_PROVIDER, "不支持的数据供应商"),
        (code::LLM_CALL_FAILED, "LLM 调用失败"),
        (code::AGENT_PROFILE_NOT_FOUND, "Agent 配置未找到"),
        (code::TOOL_CALL_FAILED, "工具调用失败"),
        (code::TOOL_NOT_CONFIGURED, "工具未配置"),
        (code::SUBWORKFLOW_FAILED, "子工作流执行失败"),
        (code::SUBWORKFLOW_NOT_CONFIGURED, "子工作流未配置"),
        (code::VECTOR_RETRIEVE_FAILED, "向量检索失败"),
        (code::VECTOR_NOT_CONFIGURED, "向量检索未配置"),
        (code::VARIABLE_NOT_FOUND, "变量未找到"),
        (code::VALIDATION_FAILED, "输入校验失败"),
        (code::PERMISSION_DENIED, "权限不足"),
        (code::TIMEOUT, "节点执行超时"),
        (code::CIRCUIT_BREAKER_OPEN, "熔断器开启（连续失败已暂停）"),
        (code::NODE_TYPE_MISMATCH, "节点类型不匹配"),
        (code::UNSUPPORTED_NODE_TYPE, "不支持的节点类型"),
        (code::IO_ERROR, "IO 错误"),
        (code::CACHE_DESERIALIZE_FAILED, "缓存反序列化失败"),
        (code::MODEL_NOT_CONFIGURED, "模型未配置"),
        (code::NODE_NOT_FOUND, "节点未找到"),
        (code::EXECUTION_CANCELLED, "执行已取消"),
    ];

    // 引擎侧英文兜底文案（engine/mod.rs 超时/熔断路径不走 NodeError Display）
    // 超时消息可能带 "(degraded: skip)" / "(degraded: useDefault)" 降级标记，用前缀匹配保留
    if let Some(rest) = error_msg.strip_prefix("Node execution timeout") {
        return format!("节点执行超时{rest}");
    }
    if error_msg == "Circuit breaker open" {
        return "熔断器开启（连续失败已暂停）".to_string();
    }

    for (code_const, label) in CODE_LABELS {
        let prefix = format!("{code_const}: ");
        if let Some(detail) = error_msg.strip_prefix(&prefix) {
            return format!("{label}：{detail}（错误码 {code_const}）");
        }
    }
    error_msg.to_string()
}

/// 判断候选决策是否"可用"：必须是对象且含非空 `action` 字符串。
///
/// V71 硬化(2026-09-11)：拦截三类"伪决策"（DB 实证导致 `decision_action` 为空的根因）：
///
///   1) portfolio-mgr 结果被包装成 `{ node_id, output: null, source, status:"terminated" }`
///      —— 取 `.output` 得到 `null`，序列化后 decision_json 字面量为 `"null"`；
///   2) 从 results 里误取到**其他节点**（如一致性检查 / 辩论收敛）的输出，
///      形状是 `{adjustedConfidence, agreementBreakdown, formulaLlmAgreement, ...}`，无 action；
///   3) `wf.output` 是 results map（键为节点 ID），整张 map 被当成决策序列化。
///
/// 三类都会让 `decision_action` 落 NULL：前端显示"决策缺失"，统计上表现为
/// "analysis completed 但无决策"。此处统一要求 action 存在，否则继续走后续兜底分支。
fn has_usable_action(v: &serde_json::Value) -> bool {
    v.get("action").and_then(|a| a.as_str()).map(|s| !s.trim().is_empty()).unwrap_or(false)
}

/// 从 rt-workflow 的节点输出中取出「实际决策对象」。
///
/// CodeNode / AgentNode 的包装形状有三种历史形态，统一在一处解包：
///   1. `{ status, result, input_params, node_id, params }` → 取 `.result`（Rhai CodeNode 标准形态）
///   2. `{ node_id, output, source, status }`               → 取 `.output`（V63 实证形态）
///   3. 已是裸决策对象                                      → 原样返回
fn unwrap_node_output(v: &serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(obj) => {
            if let Some(result) = obj.get("result") {
                result.clone()
            } else if let Some(output) = obj.get("output") {
                output.clone()
            } else {
                v.clone()
            }
        },
        _ => v.clone(),
    }
}

/// 从节点输出中取出「**纯公式侧**决策」——只认确定性节点（风控门 → 组合经理），
/// **绝不接受 LLM 兜底**（`quality-fallback`）。
///
/// 与 `extract_decision_json` 的分工（铁律 41：同一语义不得被两条链按不同口径消费）：
/// - `extract_decision_json` = **本次实际生效的最终决策**（链尾优先；D/F 档含 LLM 保守决策）
///   ⇒ 落库 / 仪表盘 / 历史列表 / 退出紧迫度。
/// - 本函数 = **公式决策**（确定性链）⇒ `compute_decision_agreement` 的「公式 vs LLM」诊断。
///
/// 2026-09-13 新增：此前 `compute_decision_agreement` 直接拿 `extract_decision_json` 当「公式侧」，
/// 一旦链尾是 LLM 兜底，同一份输出里的 `formulaAction` / `formulaRiskLevel` 就变成了
/// **LLM 的答案**却挂着「公式」的名字。实证 600031（2026-09-13）：
/// `formulaAction="减持"` / `formulaRiskLevel="高风险"` 实为 `quality-fallback` 的 LLM 输出，
/// 而真正的公式决策是风控门的 `增持` / `中风险`（11.5%）—— 前端把它渲染成
/// 「公式 ◀ 53 ▶ LLM」，用户看到的「公式」其实是 LLM。
pub(crate) fn extract_formula_decision_json(wf: &Workflow) -> Option<String> {
    for node_id in ["portfolio-risk-gate", "portfolio-mgr"] {
        let Some(node) = wf.results.get(node_id) else {
            continue;
        };
        let actual = unwrap_node_output(node);
        if has_usable_action(&actual)
            && let Ok(s) = serde_json::to_string(&actual)
        {
            return Some(s);
        }
    }
    None
}

pub(crate) fn extract_decision_json(wf: &Workflow) -> Option<String> {
    // V71: 记录 portfolio-mgr 候选不可用的原因，供后续兜底分支生成可诊断的占位决策
    let mut unusable_reason: Option<String> = None;

    // ── 优先级 0：quality-fallback（数据质量 D/F 档的 LLM 保守决策）──
    // V40 语义保持 + 2026-09-13 顺序修正：
    //   quality-gate 判 D/F（`default_case = "low-quality"`）时路由到 quality-fallback，
    //   由 AgentNode 的保守决策**替代** portfolio-mgr 的公式决策。因此本分支必须排在
    //   风控门之前 —— 风控门吃的是 portfolio-mgr 的公式结果（拓扑：
    //   portfolio-mgr → portfolio-risk-gate → rule-check → quality-gate），
    //   D/F 档下它照样会产出结果；若把 gate 排在最前，就会把「已被质量门替代的公式决策」
    //   重新抬成最终结论，直接推翻 V40 修复。
    //
    //   D/F 档的完整链路是：quality-gate(default) → quality-fallback → decision-explainer
    //   ⇒ 链尾终值是 quality-fallback，与 store-result / end-output 声明一致。
    //
    //   正常档（A/B/C）quality-fallback 为 Skipped（无 result，见 rt-workflow
    //   apply_node_status_update 仅在 result=Some 时写入）⇒ 本分支不命中，继续走 gate。
    if let Some(qf) = wf.results.get("quality-fallback") {
        if let Some(content_str) = qf.get("content").and_then(|v| v.as_str()) {
            // quality-fallback 输出格式: {"action":"持有/减持/卖出","positionPct":0-20,"confidence":20-40,"riskLevel":"高风险","reasoning":"..."}
            // P0 修复: 若 LLM 未严格遵循 prompt 缺失 confidence/riskLevel 字段，补充合理保守默认值
            if let Ok(mut v) = serde_json::from_str::<serde_json::Value>(content_str) {
                if let Some(obj) = v.as_object_mut() {
                    if !obj.contains_key("confidence") {
                        obj.insert("confidence".to_string(), json!(30.0));
                    }
                    if !obj.contains_key("riskLevel") {
                        obj.insert("riskLevel".to_string(), json!("高风险"));
                    }
                    if !obj.contains_key("decisionConfidence") {
                        if let Some(c) = obj.get("confidence").and_then(|c| c.as_f64()) {
                            obj.insert("decisionConfidence".to_string(), json!(c));
                        }
                    }
                }
                // V71: quality-fallback 也可能输出无 action 的 JSON（LLM 未遵循 schema）
                if has_usable_action(&v) {
                    return Some(v.to_string());
                }
                if unusable_reason.is_none() {
                    unusable_reason = Some("quality-fallback 输出缺少 action 字段".to_string());
                }
            }
        }
    }

    // ── 优先级 1：portfolio-risk-gate（模板声明的链尾终值）──
    // 2026-09-13 修复「风控门仓位修正被落库链整体丢弃」：
    //   模板四处（store-result.input_var / end-output.output_var / rule-check 与
    //   decision-explainer 的 contextSources）都以 portfolio-risk-gate 为终值，
    //   但本函数此前只认 portfolio-mgr ⇒ 同一次运行产出两套真相：
    //   rule-check / decision-explainer 报「已按 R-206 下调至 0%」，DB 与前端仍是
    //   portfolio-mgr 的 8.85%（实证 601166 / 2026-09-12：风险敞口被高估 8.8pt，
    //   且用户完全看不到风控曾介入）。
    //   风控门输出 = portfolio-mgr 全字段 + 覆盖 action/positionPct + risk_gate 元数据，
    //   是 portfolio-mgr 的超集，故优先取它；其缺位或不可用时再回落 portfolio-mgr
    //   （保留旧语义，兼容风控门被跳过/失败的历史运行）。
    //   ⚠️ 顺序：排在 quality-fallback **之后**。风控门吃 portfolio-mgr 的公式结果，
    //   在 D/F 档下照样产出结果；若排在 quality-fallback 之前会把「已被质量门替代的
    //   公式决策」重新抬成结论（见优先级 0 注释）。
    if let Some(gate) = wf.results.get("portfolio-risk-gate") {
        let actual = unwrap_node_output(gate);
        if has_usable_action(&actual) {
            if let Ok(s) = serde_json::to_string(&actual) {
                return Some(s);
            }
        } else {
            tracing::warn!(
                "[extract_decision_json] portfolio-risk-gate 结果缺少 action 字段，回落 portfolio-mgr"
            );
        }
    }

    // ── 优先级 2：portfolio-mgr（公式层原始决策，兼容风控门缺位的历史运行）──
    if let Some(pm) = wf.results.get("portfolio-mgr") {
        // CodeNode 包装解包（.result / .output / 裸对象），见 unwrap_node_output 注释。
        // 实际决策在 .result 字段;若 .result 缺失(旧版/异常路径)则降级用
        // 整个 pm 值,让 extract_decision_fields 至少能拿到 action 等字段。
        //
        // V63 修复: portfolio-mgr 也可能是 {node_id, output, source, status}
        // 格式（不含 result/params），决策数据在 .output 字段中。
        let actual = unwrap_node_output(pm);
        // V71: 只有含可用 action 才认作决策，否则记原因并继续兜底
        if has_usable_action(&actual) {
            if let Ok(s) = serde_json::to_string(&actual) {
                return Some(s);
            }
        } else {
            let shape: Vec<String> =
                actual.as_object().map(|o| o.keys().take(8).cloned().collect()).unwrap_or_default();
            unusable_reason =
                Some(format!("portfolio-mgr 结果缺少 action 字段(形状: {{{}}})", shape.join(",")));
            tracing::warn!(
                "[extract_decision_json] portfolio-mgr 结果不可用({}),继续尝试兜底分支",
                unusable_reason.as_deref().unwrap_or("未知")
            );
        }
    }
    // ── V57 硬化：portfolio-mgr 节点未成功产出结果时（Failed / Skipped /
    // 因上游失败被跳过 / 从未运行），rt-workflow 的 apply_node_status_update
    // 仅在 result=Some 时才写入 results（见 rt-workflow engine/mod.rs），
    // 故 results["portfolio-mgr"] 缺位，旧逻辑回退到 wf.output 会再次产出
    // "全零空壳"/决策缺失。此处携带 node_states["portfolio-mgr"].error 作可见
    // 诊断，并给出最小有效决策 action:"观望"，避免 UI 静默"决策缺失"。
    // 注意：只要节点状态不是 Completed（即没拿到有效 result）就触发，
    // 覆盖 Rhai 运行时错误(Failed)与上游失败导致 Skipped 两种空壳来源。
    if let Some(state) = wf.node_states.get("portfolio-mgr") {
        // V71: 除"节点未完成"外，节点 Completed 但结果无可辨识 action（伪决策）
        // 同样走本兜底，避免 decision_action 落 NULL。
        if state.status != NodeStatus::Completed || unusable_reason.is_some() {
            let error_msg =
                state.error.clone().or_else(|| unusable_reason.clone()).unwrap_or_else(|| {
                    match state.status {
                        NodeStatus::Skipped => {
                            "portfolio-mgr 节点被跳过（上游依赖失败导致，无本地错误详情）"
                                .to_string()
                        },
                        NodeStatus::Failed => {
                            "portfolio-mgr 节点执行失败（无错误详情）".to_string()
                        },
                        _ => "portfolio-mgr 节点未完成（状态非 Completed）".to_string(),
                    }
                });
            // 错误码资源键转可读文案：reasoning 内联可读错误，不再让用户翻
            // JSON 子字段；原始错误串（含资源键码）保留在 diagnostics.errorCode。
            let humanized = humanize_node_error(&error_msg);
            let hint = if state.status == NodeStatus::Skipped {
                "portfolio-mgr 被 Skipped：检查其上游依赖节点（trader/research-mgr/a-catalyst/t-risk 等）是否失败或超时，错误在对应 node_states[上游].error。"
            } else if state.status == NodeStatus::Completed {
                // V71: 节点跑成功但结果形状不含 action —— 检查上游 input_mapping 是否解析到空值
                "portfolio-mgr 已执行但结果无可辨识 action：优先检查该节点 input_mapping（data 键是否解析为 null）与 Rhai 是否为每一条 return 路径都输出含 action 的对象。"
            } else {
                "portfolio-mgr Rhai 运行失败：检查未走 present() 直接引用的变量、除零或类型错误。"
            };
            // P0-5(2026-09-14): 收敛到单一构造点。
            // 原先此处与 `conservative_placeholder` 是两份结构相同的手写占位
            // （仅此处多一个 errorCode），属「同一能力 N 份实现」——
            // 且两份都把「节点没产出决策」写成 action="观望"。
            return serde_json::to_string(&conservative_placeholder(
                &format!("{:?}", state.status),
                &humanized,
                hint,
                Some(error_msg.as_str()),
                &PlaceholderContext::collect(wf, Some("portfolio-mgr")),
            ))
            .ok();
        }
    }

    // 回退: workflow 顶层 output(无 output_schema 或非 stock-analysis 工作流)
    //
    // P4 修复(2026-07-25):若 wf.output 是 results map(顶层含 stock-analysis
    // 已知节点 ID 之一,如 trigger/portfolio-mgr/trader 等),说明 portfolio-mgr
    // 节点未产出有效结果且 node_states 也无记录(可能是工作流异常终止或 rt-workflow
    // 未写入 node_states 的边界场景)。此时直接序列化 wf.output 会让前端
    // normalizeDecision 拿到 results map → 识别为 results map 后因 portfolio-mgr
    // 缺失而判为"全零空壳"返回 null → 前端日志噪音 + UI 闪烁。
    //
    // 修复策略:检测到 results map 时,走最小占位结构(与 V57 硬化同款),
    // 给前端一个明确的"决策缺失,已降级为观望"信号,而非含糊的 results map。
    //
    // V71 硬化(2026-09-11): 原实现用**硬编码节点 ID 白名单**判断 results map，
    //   DB 实证漏判 —— 600900/601166/300795 的 decision_json 是 `{p-analysts, pace-calc,
    //   t-catalyst-data, t-hotmoney-data, t-lockup-data, ...}`，301302 是 `{"": {debate...}, a-catalyst,
    //   a-hotmoney, ...}`，名单里没有 `p-analysts`/`pace-calc`/空串键 → 整张 results map
    //   被当成决策序列化 → decision_action 为 NULL。
    //   改为通用判据：顶层对象**不含可用 action** 即视为 results map（决策必然含 action）。
    if let Some(output) = wf.output.as_ref() {
        let is_results_map = output.is_object() && !has_usable_action(output);
        if is_results_map {
            tracing::warn!(
                "[extract_decision_json] portfolio-mgr 缺位且 node_states 无记录,wf.output 是 results map,降级为最小占位决策"
            );
            // P0-5: 收敛到单一构造点（原为第三份手写占位）
            return serde_json::to_string(&conservative_placeholder(
                "Missing",
                "portfolio-mgr 节点在 results 和 node_states 中均缺位,可能工作流异常终止",
                "检查工作流执行日志,确认 portfolio-mgr 节点是否被正确调度。若为工作流引擎 bug,需排查 rt-workflow engine 的节点写入逻辑。",
                None,
                &PlaceholderContext::collect(wf, Some("portfolio-mgr")),
            ))
            .ok();
        }
        if has_usable_action(output) {
            serde_json::to_string(output).ok()
        } else {
            // V71: 非对象/形状异常的顶层输出（如裸字符串）同样给出可诊断占位，
            // 保证 decision_action 永不落 NULL（前端才不会显示"决策缺失"）
            serde_json::to_string(&conservative_placeholder(
                "Invalid",
                unusable_reason
                    .as_deref()
                    .unwrap_or("wf.output 形状异常且无可辨识 action"),
                "检查 wf.output 的序列化内容是否为决策对象；应为含 action 的 portfolio-mgr 结果或股票分析输出结构。",
                None,
                &PlaceholderContext::collect(wf, Some("portfolio-mgr")),
            ))
            .ok()
        }
    } else {
        None
    }
}

/// 一个被跳过的节点及其原因（A4，2026-09-14）。
#[derive(Debug, Clone)]
struct SkippedNode {
    node_id: String,
    title: String,
    /// 引擎写入的 `NodeRuntimeState::skip_reason`：
    /// `upstream_failed`（真故障）/ `upstream_skipped`（故障传播第二跳起）/
    /// `branch_not_taken`（配置意图）/ `disabled` / `unreachable`。
    reason: String,
}

/// 「决策缺失」占位决策的上下文（A4，2026-09-14）。
///
/// ## 为什么需要它
///
/// 601166 断链审计（`AUDIT-601166-chain-break-2026-09-14.md`）暴露的核心问题是
/// **「不应该继续，但应该告知错误」里的后半句完全缺失**：`bear-r3` 的 LLM 504
/// 明明记在 `node_states["bear-r3"].error` 里，却**零消费者** —— 落库只剩一句
/// 「portfolio-mgr 节点被跳过（上游依赖失败导致，无本地错误详情）」，
/// 用户拿不到任何可行动信息，只能人肉翻 62 个节点。
///
/// 本上下文把两件事补上：
/// ① `root_failure`：沿上游回溯到**第一个真正失败**的节点及其错误（真正的根因）；
/// ② `skipped_nodes`：全部被跳过节点 + 原因，让「配置意图的跳过」与
///    「故障导致的跳过」在数据层就能区分（引擎侧见 `NodeRuntimeState::skip_reason`）。
#[derive(Debug, Default, Clone)]
struct PlaceholderContext {
    root_failure: Option<(String, String)>,
    skipped_nodes: Vec<SkippedNode>,
}

impl PlaceholderContext {
    /// 采集上下文。`backtrace_from` 为回溯根因的起点节点（通常 `"portfolio-mgr"`）。
    fn collect(wf: &Workflow, backtrace_from: Option<&str>) -> Self {
        let mut skipped_nodes: Vec<SkippedNode> = wf
            .node_states
            .iter()
            .filter(|(_, s)| s.status == NodeStatus::Skipped)
            .map(|(id, s)| SkippedNode {
                node_id: id.clone(),
                title: wf
                    .nodes
                    .iter()
                    .find(|n| n.base_id() == id.as_str())
                    .map(|n| n.base_title().to_string())
                    .unwrap_or_default(),
                reason: s.skip_reason.clone().unwrap_or_else(|| "unknown".to_string()),
            })
            .collect();
        // HashMap 迭代顺序随机 ⇒ 显式排序，保证同一份运行产物可复现、可 diff。
        skipped_nodes.sort_by(|a, b| a.node_id.cmp(&b.node_id));
        let root_failure = backtrace_from.and_then(|start| find_root_failure(wf, start));
        Self { root_failure, skipped_nodes }
    }
}

/// 从 `start` 沿**入边**做 BFS，返回第一个 `Failed` 节点及其错误。
///
/// BFS 保证「离 `start` 最近」优先 —— 在故障传播语义上这就是最直接的原因
/// （更远的 `Failed` 往往是它的下游受害者）。同层多个失败时按边的声明顺序取首个，
/// 结果确定。找不到返回 `None`（如纯分支跳过，根本没节点失败）。
fn find_root_failure(wf: &Workflow, start: &str) -> Option<(String, String)> {
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut queue: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    visited.insert(start.to_string());
    queue.push_back(start.to_string());
    while let Some(cur) = queue.pop_front() {
        for e in wf.edges.iter() {
            if e.target != cur || !visited.insert(e.source.clone()) {
                continue;
            }
            if let Some(st) = wf.node_states.get(&e.source)
                && st.status == NodeStatus::Failed
            {
                let err = st
                    .error
                    .clone()
                    .unwrap_or_else(|| "（节点状态为 Failed 但未记录错误详情）".to_string());
                return Some((e.source.clone(), err));
            }
            queue.push_back(e.source.clone());
        }
    }
    None
}

/// 构造「决策缺失」占位决策（action=数据缺失 / 0 仓位 / 带 diagnostics）。
///
/// V71 抽取：原三处兜底分支各自手写一份相同结构的占位 JSON，
/// 统一为单一构造点，避免后续再出现"某条兜底路径漏写 action"。
///
/// ⚠️ P0-5(2026-09-14): action 由「观望」改为显式缺失哨兵「数据缺失」。
/// 原实现（action="观望" + positionPct=0 + confidence=0）让「节点根本没产出决策」
/// 在 UI 与 DB 上表现为一条**已做出的观望结论**，与真实观望无法区分 ——
/// 缺失被两种业务语义（观望 / uncertain）分别吸收，且无法事后统计。
/// 现填哨兵，前端渲染「数据缺失」；归因信息完整保留在 `diagnostics`。
///
/// A4(2026-09-14)：新增 `ctx`，把**根因失败节点**与**跳过清单**带进 reasoning 与
/// diagnostics。旧文案「上游依赖失败导致，无本地错误详情」是本项目最典型的
/// 「故障不可观测」形态 —— 错误就在内存里，只是没有消费者。
fn conservative_placeholder(
    node_status: &str,
    node_error: &str,
    hint: &str,
    error_code: Option<&str>,
    ctx: &PlaceholderContext,
) -> serde_json::Value {
    let mut fallback = serde_json::Map::new();
    fallback.insert(
        "action".to_string(),
        json!(axagent_analysis_engine::decision_action::ActionKind::Unavailable.as_storage_cn()),
    );
    fallback.insert("positionPct".to_string(), json!(0));
    fallback.insert("confidence".to_string(), json!(0));
    fallback.insert("riskLevel".to_string(), json!("未知"));
    fallback.insert("timeHorizon".to_string(), json!("短期"));
    // 用户可读的 reasoning：优先讲**根因**，其次才是本节点自己的状态。
    let reasoning = match (&ctx.root_failure, ctx.skipped_nodes.len()) {
        (Some((nid, err)), n) if n > 0 => format!(
            "组合管理节点未产出有效决策（数据缺失，非观望结论）。根因：上游节点 `{nid}` 执行失败 —— {err}。\
             该失败已被引擎判定为不可继续，{n} 个下游节点随即被级联跳过（含 portfolio-mgr），因此本次没有决策。"
        ),
        (Some((nid, err)), _) => format!(
            "组合管理节点未产出有效决策（数据缺失，非观望结论）。根因：上游节点 `{nid}` 执行失败 —— {err}。"
        ),
        (None, 0) => {
            format!("组合管理节点未产出有效决策（数据缺失，非观望结论）。原因：{node_error}")
        },
        (None, n) => format!(
            "组合管理节点未产出有效决策（数据缺失，非观望结论）。原因：{node_error}；\
             另有 {n} 个节点被跳过（未发现 Failed 节点，疑为分支未选中或上游越界）。"
        ),
    };
    fallback.insert("reasoning".to_string(), json!(reasoning));
    let mut diag = serde_json::Map::new();
    diag.insert("node".to_string(), json!("portfolio-mgr"));
    diag.insert("nodeStatus".to_string(), json!(node_status));
    diag.insert("nodeError".to_string(), json!(node_error));
    diag.insert("hint".to_string(), json!(hint));
    if let Some(code) = error_code {
        diag.insert("errorCode".to_string(), json!(code));
    }
    // A4：根因 + 跳过清单（结构化，供前端分栏展示与事后统计）
    if let Some((nid, err)) = &ctx.root_failure {
        diag.insert("rootFailureNode".to_string(), json!(nid));
        diag.insert("rootFailureError".to_string(), json!(err));
    }
    if !ctx.skipped_nodes.is_empty() {
        diag.insert("skippedNodeCount".to_string(), json!(ctx.skipped_nodes.len()));
        diag.insert(
            "skippedNodes".to_string(),
            json!(
                ctx.skipped_nodes
                    .iter()
                    .map(|s| json!({
                        "nodeId": s.node_id,
                        "title": s.title,
                        "reason": s.reason,
                    }))
                    .collect::<Vec<_>>()
            ),
        );
    }
    fallback.insert("diagnostics".to_string(), serde_json::Value::Object(diag));
    serde_json::Value::Object(fallback)
}

/// 从 Workflow 结果中提取 trader 节点的 LLM 决策 JSON。
///
/// trader 节点输出格式:
/// ```json
/// { "stance": "买入", "positionPct": 35, "confidence": 0.72,
///   "summary": "...", "key_points": [...], "scenarios": [...] }
/// ```
///
/// 用作"方案 D 双向并存"的 LLM 视角,与 portfolio-mgr 公式视角对比。
/// 优先取 `results["trader"]["result"]`（AgentNode 包装内的实际输出），
/// 回退到 `results["trader"]` 本身。
pub(crate) fn extract_llm_decision_json(wf: &Workflow) -> Option<String> {
    let trader = wf.results.get("trader")?;
    // V37 修复: trader 是 AgentNode，输出结构为 {role, content: <json_string>, ...}，
    // LLM 的业务字段（action/targetPrice/confidence）在 content JSON 字符串内部。
    // 旧代码取 .result（CodeNode 的字段），AgentNode 无此字段→永远 fallback 到包装对象，
    // 导致 compute_decision_agreement 拿不到 action 字段，一致性分数走兜底。
    // V41 修复: content 是 JSON 字符串，需解析为 JSON 对象再序列化后存储。
    // 旧代码直接 serialize Value::String(content)，导致 DB 中存储的是双重嵌套
    // 的 JSON 字符串（前端 JSON.parse 后仍是字符串而非对象）。
    match trader {
        serde_json::Value::Object(obj) => {
            if let Some(content_str) = obj.get("content").and_then(|v| v.as_str()) {
                // 解析 content 内层 JSON 字符串为 JSON 对象，再序列化
                if let Ok(mut parsed) = serde_json::from_str::<serde_json::Value>(content_str) {
                    // V60 修复: 展开 report 字段的嵌套 JSON
                    // LLM 可能输出 {report: '{...}'} 格式，实际决策数据（verdict/currentPrice/confidence 等）
                    // 在 report 值的 JSON 字符串内部。将其展开到顶层，使前端 extractLlmField 能直接读取。
                    if let Some(report_str) = parsed.get("report").and_then(|v| v.as_str()) {
                        if let Ok(report_parsed) =
                            serde_json::from_str::<serde_json::Value>(report_str)
                        {
                            if let Some(report_obj) = report_parsed.as_object() {
                                let obj = parsed.as_object_mut().expect("parsed is already Object");
                                for (k, v) in report_obj {
                                    // 不覆盖顶层已有的同名字段
                                    obj.entry(k).or_insert_with(|| v.clone());
                                }
                            }
                        }
                    }
                    // V64 修复: report 展开后, 若顶层有 verdict 无 action,
                    // 将 verdict 映射为 action (trader 输出方向标签而非操作指令)
                    let needs_action = parsed.get("action").and_then(|v| v.as_str()).is_none();
                    if needs_action {
                        // 先 clone verdict 值, 避免与后续 mutable borrow 冲突
                        let verdict_clone =
                            parsed.get("verdict").and_then(|v| v.as_str()).map(|s| s.to_string());
                        if let Some(ref v) = verdict_clone {
                            // P1-1(2026-09-14): verdict → action 改走统一降维映射
                            // (`decision_action::verdict_to_action`)，不再内联第二份。
                            // ⚠️ 语义修正：「不确定 / 无法判断」现映射为「不确定」而**不是**
                            // 「观望」—— 方向未知 ≠ 判断为中性，压成观望等于替 LLM 下结论。
                            let mapped =
                                axagent_analysis_engine::decision_action::verdict_to_action(v)
                                    .map(|k| k.as_storage_cn())
                                    .unwrap_or_else(|| v.trim());
                            if let Some(obj) = parsed.as_object_mut() {
                                obj.insert(
                                    "action".into(),
                                    serde_json::Value::String(mapped.to_string()),
                                );
                            }
                        }
                    }
                    // V46 修复: 标准化 LLM 输出的 action 字段
                    // trader prompt 规定 action ∈ {买入,增持,持有,减持,卖出,观望},
                    // 但 LLM 可能输出"不确定""未知"等非标准值（尤其是当数据矛盾时
                    // LLM 选择输出"不确定"作为逃逸）。
                    // 通过白名单强制映射, 防止 DB 和 UI 出现非标准值。
                    // 注意: 不修改 targetPrice/stopLoss/confidence 等数值字段,
                    // 它们错误时 portfolio-mgr 的 sanity 预检会兜底。
                    normalize_llm_action(&mut parsed);
                    return serde_json::to_string(&parsed).ok();
                }
                // 解析失败时回退：返回原始 content 字符串
                return Some(content_str.to_string());
            }
            serde_json::to_string(trader).ok()
        },
        _ => serde_json::to_string(trader).ok(),
    }
}

/// 标准化 LLM 输出的 action 字段, 把非标准值映射到标准值。
///
/// 标准值: 买入 / 增持 / 持有 / 减持 / 卖出 / 观望（+ 不确定）
///
/// ⚠️ P1-1(2026-09-14): 值域权威表已收敛到 `decision_action::normalize_action`，
/// 本函数不再内联维护第二份。
///
/// ⚠️ 语义修正：`"不确定" / "未知" / "" / "无法判断"` 现归入 **「不确定」**，
/// 而不再是原先的「观望」。原实现把「LLM 说不清」压成「观望（不操作）」，
/// 等于替 LLM 下了一个它没下的结论，前端据此渲染的观望标签是**伪造的结论**。
/// 兜底分支同理：未识别值不再默认「观望」。
pub(crate) fn normalize_llm_action(parsed: &mut serde_json::Value) {
    use axagent_analysis_engine::decision_action::{ActionKind, normalize_action};

    let Some(obj) = parsed.as_object_mut() else {
        return;
    };
    let trimmed = match obj.get("action").and_then(|v| v.as_str()) {
        Some(a) => a.trim().to_string(),
        None => return,
    };
    // 已在标准值域内（含「观望」「不确定」）→ 不处理
    if normalize_action(&trimmed).is_some() {
        return;
    }
    // V46 映射表的近义词部分：LLM 自由发挥的简写 / 极端表述
    let mapped: ActionKind = match trimmed.as_str() {
        // 无判断（含空值）→ 不确定。**不是**观望
        "未知" | "?" | "??" | "" | "无法确定" => ActionKind::Uncertain,
        // 明确看空 → 卖出
        "回避" | "远离" | "清仓" | "止损" | "割肉" | "离场" => ActionKind::Sell,
        // 近义词映射
        "卖" | "sell" | "做空" | "空" => ActionKind::Sell,
        "买" | "buy" | "做多" | "多" => ActionKind::Buy,
        "减" => ActionKind::Reduce,
        "增" | "加" => ActionKind::Increase,
        "持" => ActionKind::Hold,
        "观" => ActionKind::Wait,
        // 兜底: 未识别值 → 不确定（原实现压成「观望」，属伪造结论）
        _ => {
            tracing::warn!(
                "[normalize_llm_action] 未识别 action 值 {:?}, 归入「不确定」（不再默认观望）",
                trimmed
            );
            ActionKind::Uncertain
        },
    };
    obj.insert("action".to_string(), serde_json::Value::String(mapped.as_storage_cn().to_string()));
}

/// 双视角一致性诊断结果
///
/// V65 升级: 从 3 维度（action 50/positionPct 30/confidence 20）扩展为 6 维度
///   (action 30 + positionPct 20 + confidence 15 + riskLevel 15 + data_gaps 10 + evidence_cited 10)
/// 总分 100，低于 60 触发人工复核。
///
/// **2026-09-21 口径变更**：移除 `data_gaps` 维度 ⇒ 现为 **5 维度**，
///   内部满分 90、输出时归一化回 100 制（见 `compute_decision_agreement` 注释）。
///   移除理由：两侧缺口清单**命名体系不可比** —— 公式侧由 `portfolio-mgr.rhai`
///   机械枚举字段缺失（`PE数据(t-risk)` 这类固定标签），LLM 侧是 trader 自由文本
///   自述（实测 601166 8 条 / 688114 6 条语义缺口）⇒ Jaccard 交集恒 0，且分母含
///   LLM 侧条数 ⇒「LLM 越诚实列缺口，这一项扣得越狠」。DB 实证（50 条决策）：
///   16 条有缺口、其中 12 条该维度 0 分，全样本均值 1.8/10；并且会把 action /
///   position / riskLevel **全部一致**的一轮（688114：三路 action 均「观望」）
///   判成 `data_gaps_diverge` 冲突类型、压低 `adjustedConfidence`。
///
/// 上层可根据维度详情:
///   - 决定 confidence 调制幅度
///   - 生成分歧诊断 reasoning 文本
///   - 判断是否触发人工复核
///
/// P0 修复: 保留 f7 自指污染标记字段，标注公式决策中 trader 因子(f7)的参与程度，
/// 帮助识别"公式已含 trader 观点"导致一致性虚高或逻辑矛盾。
pub(crate) struct AgreementBreakdown {
    /// 总分 0-100（5 维度加权，内部满分 90 归一化）
    pub total: i32,
    /// action 维度原始分 (满分 30)
    pub action_score: f64,
    /// action 是否基本一致 (>= 20 分)
    pub action_ok: bool,
    /// action 一致性说明 (exact_match / same_direction / opposite / ...)
    pub action_note: String,
    /// 公式视角的 action 原始值
    pub formula_action: String,
    /// LLM 视角的 action 原始值
    pub llm_action: String,
    /// positionPct 维度原始分 (满分 20)
    pub position_score: f64,
    /// 仓位差值绝对值
    pub position_gap: Option<f64>,
    /// confidence 维度原始分 (满分 15)
    pub confidence_score: f64,
    /// 置信度差值绝对值
    pub confidence_gap: Option<f64>,
    /// V65 新增: riskLevel 维度原始分 (满分 15)
    pub risk_level_score: f64,
    /// V65 新增: 公式 riskLevel 原始值
    pub formula_risk_level: String,
    /// V65 新增: LLM riskLevel 原始值
    pub llm_risk_level: String,
    /// V65 新增: evidence_cited 维度原始分 (满分 10)
    pub evidence_score: f64,
    /// V65 新增: LLM 引用上游论据数量
    pub evidence_count: i32,
    /// 冲突类型: all_agree / opposite_direction / action_divergence / position_gap / confidence_gap / risk_gap
    /// （`data_gaps_diverge` 已于 2026-09-21 随 data_gaps 维度一并移除）
    pub conflict_type: String,
    // ── P0: f7 自指污染标记（向后兼容保留）──
    /// 公式决策中 f7（trader 因子）权重占总权重百分比。None=无 f7 数据。
    pub f7_weight_pct: Option<f64>,
    /// 排除 f7 后的"纯净"后验值（0~1）。None=无 f7 数据。
    pub f7_free_posterior: Option<f64>,
    /// 排除 f7 后的"纯净"action。None=无 f7 数据。
    pub f7_free_action: Option<String>,
    /// 无 f7 版本的 action 一致性原始分 (满分 30，与主 action_score 相同语义)
    pub f7_free_action_score: Option<f64>,
}

/// 计算公式决策与 LLM 决策的一致性分数（0-100）。
///
/// V65 升级：维度对比，对应 trader.md 中"双视角对比说明"的权重分配：
///   - action: 30 分（精确匹配 30 / 同向 20 / 中性不同义 5-10 / 对立 0）
///   - positionPct: 20 分（≤10% 满分 20 / ≤20% 半分 10 / >20% 零分 0）
///   - confidence: 15 分（差值 ≤10 满分 15 / ≤20 半分 10 / 否则 5）
///   - riskLevel: 15 分（精确匹配 15 / 相邻 8 / 跨级 0）
///   - evidence 引用密度: 10 分（≥3 条满分 10 / 2 条 5 / <2 条 0）
///     ⇒ 内部满分 **90**，出口统一归一化回 100 制（`raw / 90 × 100`），
///     使 50 分仍为中性 —— `core.rs` 的 `factor = 1 + (total-50)/100` 与前端
///     60 分档位文案因此**无需改动**。
///
/// **2026-09-21 移除 `data_gaps` 维度**：两侧缺口清单命名体系不可比
///   （公式侧 = rhai 机械枚举字段缺失，LLM 侧 = trader 自由文本自述），
///   Jaccard 在该前提下不是「一致性」的任何正确度量 —— 实测恒 0 分，
///   且分母含 LLM 侧条数 ⇒ 惩罚的恰是 LLM 的坦诚度。缺口信息本身仍照旧输出
///   （`decision.data_gaps` → UI 提示），只是不再进这张评分表。
///
/// 归一化规则（与前端 normalizeAction 保持一致）:
/// - 移除空格/斜杠/下划线/全角空格
/// - 小写比较
/// - "买"和"增持"视为一致，"卖"和"减持"视为一致
pub(crate) fn compute_decision_agreement(
    formula_json: Option<&str>,
    llm_json: Option<&str>,
) -> Option<AgreementBreakdown> {
    let fj = serde_json::from_str::<serde_json::Value>(formula_json?).ok()?;
    let lj = serde_json::from_str::<serde_json::Value>(llm_json?).ok()?;

    // 归一化操作字符串
    let norm = |s: &str| s.trim().to_lowercase().replace([' ', '/', '_', '\u{3000}'], "");

    // ── 公式字段 ──
    let f_action = fj.get("action").and_then(|v| v.as_str().map(norm));
    let f_pos = fj.get("positionPct").and_then(|v| v.as_f64());
    let f_conf = fj.get("confidence").and_then(|v| v.as_f64());
    // V65: 公式 riskLevel（data_gaps 解析已于 2026-09-21 随该维度一并移除）
    let f_risk = fj.get("riskLevel").and_then(|v| v.as_str()).map(norm).unwrap_or_default();

    // ── LLM 字段（V65: trader 现在输出完整字段）──
    let l_action = lj.get("action").and_then(|v| v.as_str().map(norm));
    let l_pos = lj.get("positionPct").and_then(|v| v.as_f64());
    let l_conf = lj.get("confidence").and_then(|v| v.as_f64());
    let l_risk = lj.get("riskLevel").and_then(|v| v.as_str()).map(norm).unwrap_or_default();
    // V65: evidence_cited 数量
    let evidence_count: i32 = lj
        .get("evidence_cited")
        .and_then(|v| v.as_array())
        .map(|arr| arr.len() as i32)
        .unwrap_or(0);

    // V50: 保存原始 action 值用于诊断展示
    let f_action_raw = fj.get("action").and_then(|v| v.as_str()).unwrap_or("?").to_string();
    let l_action_raw = lj.get("action").and_then(|v| v.as_str()).unwrap_or("?").to_string();
    // 预计算维度差值
    let pos_gap: Option<f64> = match (f_pos, l_pos) {
        (Some(a), Some(b)) => Some((a - b).abs()),
        _ => None,
    };
    let conf_gap: Option<f64> = match (f_conf, l_conf) {
        (Some(a), Some(b)) => Some((a - b).abs()),
        _ => None,
    };

    // ── V65: action 评分（满分 30，原满分 50 的 60%）──
    // 精确匹配 30 > 同向同类 20 > 中性不同义 5-10 > 对立 0
    //
    // P1-6(2026-09-14): 判定改走统一归一化（`decision_action::normalize_action`）。
    // 原实现用中文字符包含 / 全等判定：链路上 action 一旦是英文（BUY / HOLD / WAIT）
    // 或 dashboard 值域短语，`is_buy`/`is_sell`/`is_hold` 全不命中 ⇒ 同向双方被
    // 误判成「对立方向 0 分」（一致性分数虚假偏低）。
    //
    // ⚠️ 同时修掉两处**不可达 / 判据重叠**：
    //   ① 原 `(持有|观望) vs 不确定 = 3` 已包含 is_watch 分支，紧随其后的
    //      `观望 vs 不确定 = 6` 永远命中不到（同一判据被两条分支共用，靠前的吞掉靠后的）；
    //   ② 无「缺失哨兵」分支 ⇒ 哨兵会被当英文串落入「对立 0 分」。
    // 现按原**声明分值**保留语义（持有 vs 不确定 = 3、观望 vs 不确定 = 6，观望更接近
    // 「说不好」），并使各分支真正互斥；缺失哨兵按「单侧缺失」档处理。
    use axagent_analysis_engine::decision_action::{ActionKind, normalize_action};
    let f_kind = f_action.as_deref().and_then(normalize_action);
    let l_kind = l_action.as_deref().and_then(normalize_action);
    let is_bull = |k: ActionKind| matches!(k, ActionKind::Buy | ActionKind::Increase);
    let is_bear = |k: ActionKind| matches!(k, ActionKind::Sell | ActionKind::Reduce);
    let action_score: f64 = match (f_kind, l_kind) {
        (Some(a), Some(b)) if a == b => 30.0,
        (Some(a), Some(b)) if is_bull(a) && is_bull(b) => 20.0,
        (Some(a), Some(b)) if is_bear(a) && is_bear(b) => 20.0,
        // 中性但不同义: 持有 vs 观望 = 10
        (Some(ActionKind::Hold), Some(ActionKind::Wait))
        | (Some(ActionKind::Wait), Some(ActionKind::Hold)) => 10.0,
        // 观望 vs 不确定 = 6（观望与「说不好」比持有更接近）
        (Some(ActionKind::Wait), Some(ActionKind::Uncertain))
        | (Some(ActionKind::Uncertain), Some(ActionKind::Wait)) => 6.0,
        // 明确中性 vs 不确定: 持有 vs 不确定 = 3
        (Some(ActionKind::Hold), Some(ActionKind::Uncertain))
        | (Some(ActionKind::Uncertain), Some(ActionKind::Hold)) => 3.0,
        // 缺失哨兵不是方向判断，不参与方向比对（与「单侧缺失」同档）
        (Some(ActionKind::Unavailable), Some(_)) | (Some(_), Some(ActionKind::Unavailable)) => 15.0,
        // 对立方向
        (Some(_), Some(_)) => 0.0,
        // 单侧缺失 / 未识别值域
        _ => 15.0,
    };

    // ── V65: positionPct 评分（满分 20，原满分 30 的 2/3）──
    let pos_score: f64 = match (f_pos, l_pos) {
        (Some(a), Some(b)) => {
            let diff = (a - b).abs();
            if diff <= 10.0 {
                20.0
            } else if diff <= 20.0 {
                10.0
            } else {
                0.0
            }
        },
        // V65: 单侧缺失不再给兜底分（避免虚高），记 0
        _ => 0.0,
    };

    // ── V65: confidence 评分（满分 15，原满分 20 的 75%）──
    // 注意：trader 的 confidence 是 0-100 整数，公式也是 0-100
    let conf_score: f64 = match (f_conf, l_conf) {
        (Some(a), Some(b)) => {
            let diff = (a - b).abs();
            if diff <= 10.0 {
                15.0
            } else if diff <= 20.0 {
                10.0
            } else if diff <= 40.0 {
                5.0
            } else {
                0.0
            }
        },
        _ => 0.0,
    };

    // ── V65: riskLevel 评分（满分 15）──
    // 公式与 LLM 都输出 4 档风险等级，比较等级距离
    let risk_rank = |s: &str| -> i32 {
        match s {
            s if s.contains("低") => 0,
            s if s.contains("中") => 1,
            s if s.contains("高") && !s.contains("极高") => 2,
            s if s.contains("极高") => 3,
            _ => 1, // 默认中风险
        }
    };
    let f_risk_rank = risk_rank(&f_risk);
    let l_risk_rank = risk_rank(&l_risk);
    let risk_diff = (f_risk_rank - l_risk_rank).abs();
    let risk_level_score: f64 = match risk_diff {
        0 => 15.0, // 精确匹配
        1 => 8.0,  // 相邻
        _ => 0.0,  // 跨级
    };
    let f_risk_raw = fj.get("riskLevel").and_then(|v| v.as_str()).unwrap_or("?").to_string();
    let l_risk_raw = lj.get("riskLevel").and_then(|v| v.as_str()).unwrap_or("?").to_string();

    // ── 2026-09-21 移除 data_gaps 评分段 ──
    // 原实现（V75 起）：双方非空 → Jaccard×10 / 仅一侧为空 → 中性 5 / 双方为空 → 10。
    // 删除理由见本函数文档与 `AgreementBreakdown` 的结构体注释：两侧缺口清单
    // **命名体系不可比**（公式侧 = rhai 机械枚举字段缺失，LLM 侧 = trader 自由文本
    // 自述），Jaccard 恒 0；DB 实证：16 条有缺口的决策里 12 条该维度 0 分，且会把
    // 方向完全一致的一轮判成 `data_gaps_diverge` 假冲突。
    // 缺口数据本身**未丢失** —— 仍由 `decision.data_gaps` 输出给 UI（DecisionTrustNotice）。

    // ── V65: evidence_cited 评分（满分 10）──
    // ≥3 条满分 10 / 2 条 5 / <2 条 0
    let evidence_score: f64 = match evidence_count {
        n if n >= 3 => 10.0,
        2 => 5.0,
        _ => 0.0,
    };

    // ── 5 维度加权总分（内部满分 90）→ 归一化回 100 制 ──
    // 2026-09-21: 移除 data_gaps 维度（理由见函数文档）。此处**必须**归一化：
    //   `total` 既是 `core.rs` 的 `factor = 1 + (total-50)/100` 的输入，也是前端
    //   60 分档位文案的判据 —— 若直接输出 90 制，50 不再是中性点，且新旧记录的
    //   「一致性」数值不可比。
    const RAW_MAX: f64 = 90.0;
    let raw_total = action_score + pos_score + conf_score + risk_level_score + evidence_score;
    let total = (raw_total / RAW_MAX * 100.0).round() as i32;

    // ── P0: 从公式决策中提取 f7_free 信息（消除自指悖论）──
    let f7_free_info = fj.get("f7_free").and_then(|v| {
        if v.is_object() {
            let obj = v.as_object()?;
            let f7_weight = obj.get("f7_weight").and_then(|w| w.as_f64())?;
            let total_weight = obj.get("total_weight").and_then(|w| w.as_f64())?;
            let f7_weight_pct = if total_weight > 0.0 {
                Some((f7_weight / total_weight * 100.0 * 10.0).round() / 10.0)
            } else {
                None
            };
            let posterior = obj.get("posterior").and_then(|p| p.as_f64());
            let action = obj.get("action").and_then(|a| a.as_str().map(|s| s.to_string()));
            Some((f7_weight_pct, posterior, action))
        } else {
            None
        }
    });
    let (f7_weight_pct, f7_free_posterior, f7_free_action) =
        f7_free_info.unwrap_or((None, None, None));

    // 计算无 f7 版本的 action 一致性评分（与主 action_score 同尺度，满分 30）
    // P1-6(2026-09-14): 与主 action_score 同源，改走 `decision_action::normalize_action`。
    //   原实现走 `f7_free_action.map(norm)` + 一组局部 `is_buy/is_sell/is_hold/is_watch/
    //   is_uncertain` 逐词判定；该组判据的值域只有中文与极少数英文缩写 ⇒ 一旦上游是
    //   dashboard 值域（`strong_buy` / `WAIT` 等）全部不命中，直接落到
    //   `(Some(_), Some(_)) => Some(0.0)` —— 同向双方被判成「完全对立」并计入冲突类型分类。
    //   另：原 `(持有|观望) vs 不确定 = 3` 已把「观望」吃掉，紧随其后的
    //   `(观望 vs 不确定) = 6` **永不命中**（声明了却不可达的分支）。
    //   归并后按 `ActionKind` 穷举，并按声明分值重建为互斥分支（wait 在前 6 / hold 在后 3）。
    let f7_compare_target = l_action.as_deref().or(f_action.as_deref());
    let f7_kind = f7_free_action.as_deref().and_then(normalize_action);
    let tgt_kind = f7_compare_target.and_then(normalize_action);
    let is_bull_k = |k: ActionKind| matches!(k, ActionKind::Buy | ActionKind::Increase);
    let is_bear_k = |k: ActionKind| matches!(k, ActionKind::Sell | ActionKind::Reduce);
    let f7_free_action_score = match (f7_kind, tgt_kind) {
        (Some(a), Some(b)) if a == b => Some(30.0),
        (Some(a), Some(b)) if is_bull_k(a) && is_bull_k(b) => Some(20.0),
        (Some(a), Some(b)) if is_bear_k(a) && is_bear_k(b) => Some(20.0),
        (Some(ActionKind::Hold), Some(ActionKind::Wait))
        | (Some(ActionKind::Wait), Some(ActionKind::Hold)) => Some(10.0),
        (Some(ActionKind::Wait), Some(ActionKind::Uncertain))
        | (Some(ActionKind::Uncertain), Some(ActionKind::Wait)) => Some(6.0),
        (Some(ActionKind::Hold), Some(ActionKind::Uncertain))
        | (Some(ActionKind::Uncertain), Some(ActionKind::Hold)) => Some(3.0),
        (Some(_), Some(_)) => Some(0.0),
        _ => None,
    };

    // ── 冲突类型分类（5 维度版；2026-09-21 移除 data_gaps_diverge 分支）──
    let conflict_type: &str = if l_action.is_none() && evidence_count == 0 {
        // 完全无 LLM 视角输入
        match f7_free_action_score {
            Some(s) if s >= 25.0 => "f7_low_influence",
            Some(s) if s >= 15.0 => "f7_moderate_influence",
            Some(s) if s >= 8.0 => "f7_high_influence",
            _ => "f7_dominant",
        }
    } else if action_score >= 25.0
        && pos_score >= 15.0
        && conf_score >= 12.0
        && risk_level_score >= 12.0
    {
        "all_agree"
    } else if action_score == 0.0 {
        "opposite_direction"
    } else if action_score <= 5.0 {
        "action_divergence"
    } else if pos_score == 0.0 && f_pos.is_some() && l_pos.is_some() {
        "position_gap"
    } else if risk_level_score == 0.0 && !f_risk.is_empty() && !l_risk.is_empty() {
        "risk_gap"
    } else {
        "confidence_gap"
    };
    // action_note 分类
    let action_note: &str = if action_score >= 30.0 {
        "exact_match"
    } else if action_score >= 20.0 {
        "same_direction"
    } else if action_score >= 10.0 {
        "hold_vs_watch"
    } else if action_score >= 6.0 {
        "watch_vs_uncertain"
    } else if action_score >= 3.0 {
        "definite_vs_uncertain"
    } else if action_score == 0.0 {
        "opposite"
    } else {
        "missing_one_side"
    };

    Some(AgreementBreakdown {
        total,
        action_score,
        action_ok: action_score >= 20.0,
        action_note: action_note.to_string(),
        formula_action: f_action_raw,
        llm_action: l_action_raw,
        position_score: pos_score,
        position_gap: pos_gap,
        confidence_score: conf_score,
        confidence_gap: conf_gap,
        risk_level_score,
        formula_risk_level: f_risk_raw,
        llm_risk_level: l_risk_raw,
        evidence_score,
        evidence_count,
        conflict_type: conflict_type.to_string(),
        f7_weight_pct,
        f7_free_posterior,
        f7_free_action,
        f7_free_action_score,
    })
}

/// 解析 as_of_date 入参：None/空串 → None（live），Some(s) → 解析为 AsOfContext
/// 抽出供单测：未来日期 / 错误格式必须 4xx-style 错误
pub(crate) fn parse_asof_param(s: Option<String>) -> Result<Option<AsOfContext>, String> {
    AsOfContext::parse_optional(s.as_deref())
}

/// 默认值，与 stock-analysis 模板的 defaults 保持一致；
/// 改动这里请同步 `StockAnalysisConfigPanel.getDefaultVariables()`。
/// V39 修复: 从 300s 提升到 600s，适配 max_tool_rounds=3 的多轮工具节点
/// （trader/research-mgr 等节点 3 轮 LLM+工具调用总耗时约 200-400s）。
const DEFAULT_MAX_CONCURRENT: usize = 8;
const DEFAULT_STEP_TIMEOUT_SECS: u64 = 600;
/// 工作流整体超时（秒）。单步 step_timeout 只限单节点，多步累计可能很久；
/// 总超时兜底防止 LLM 卡死或 vendor 长时间无响应导致分析永久挂起。
/// 默认 30 分钟 = 1800s，覆盖典型 10+ 节点工作流（每步 600s × 并发 8 的最坏路径）。
const DEFAULT_TOTAL_TIMEOUT_SECS: u64 = 1800;

/// 从模板 variables 中解析 RunOptions 关键参数。
///
/// 用户在「股票分析设置 → 参数」中调整 `max_concurrent` /
/// `agent_timeout_secs` / `total_timeout_secs` 后，这里读到的就是新值；
/// 如果模板里没有这些 key（旧版本 / 用户清空）则用默认值。
///
/// 容错策略：
///   * 越界 / 非法类型 → 用默认值；
///   * max_concurrent ∈ [1, 32]，过小会让并发退化为串行，过大会拖垮 LLM 速率。
///   * step_timeout ∈ [10, 3600] 秒，避免 0 或极端大值。
///   * total_timeout ∈ [60, 7200] 秒，下限 1 分钟，上限 2 小时。
pub(crate) fn resolve_runtime_options(
    variables: Option<&[axagent_harness::workflow_types::Variable]>,
) -> (usize, std::time::Duration, std::time::Duration) {
    let lookup = |name: &str| -> Option<serde_json::Value> {
        variables.and_then(|vs| vs.iter().find(|v| v.name == name)).map(|v| v.value.clone())
    };

    let max_concurrent = lookup("max_concurrent")
        .and_then(|v| v.as_u64())
        .map(|n| n.clamp(1, 32) as usize)
        .unwrap_or(DEFAULT_MAX_CONCURRENT);

    let step_timeout_secs = lookup("agent_timeout_secs")
        .and_then(|v| v.as_u64())
        .map(|n| n.clamp(10, 3600))
        .unwrap_or(DEFAULT_STEP_TIMEOUT_SECS);

    let total_timeout_secs = lookup("total_timeout_secs")
        .and_then(|v| v.as_u64())
        .map(|n| n.clamp(60, 7200))
        .unwrap_or(DEFAULT_TOTAL_TIMEOUT_SECS);

    (
        max_concurrent,
        std::time::Duration::from_secs(step_timeout_secs),
        std::time::Duration::from_secs(total_timeout_secs),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axagent_harness::workflow_types::Variable;
    use serde_json::json;

    /// 回归测试：t-scoring 三种真实包装形态必须能剥到 {total, signal}（600089 实证）。
    #[test]
    fn unwrap_tool_node_content_peels_real_tscoring_shapes() {
        let scoring = json!({"total": 62, "signal": "hold", "currentPrice": 19.01});
        let scoring_str = serde_json::to_string(&scoring).unwrap();

        // 形态 1：顶层 t-scoring = JSON 字符串 {"node_id","result":{"content":"<scoring>"}}
        let top_level_string = json!({
            "node_id": "t-scoring",
            "result": { "content": scoring_str.clone() }
        })
        .to_string();
        // 形态 2：_raw.t-scoring = {node_id, result:{content}, tool_name}
        let raw_form = json!({
            "node_id": "t-scoring",
            "result": { "content": scoring_str.clone() },
            "tool_name": "compute_scoring"
        });
        // 形态 3：result.t-scoring = {content: "<scoring>"}
        let remapped_form = json!({ "content": scoring_str.clone() });
        // 形态 4：已解包的业务对象原样返回
        for shape in
            [serde_json::Value::String(top_level_string), raw_form, remapped_form, scoring.clone()]
        {
            let got = unwrap_tool_node_content(&shape);
            assert_eq!(got.get("total").and_then(|v| v.as_u64()), Some(62));
            assert_eq!(got.get("signal").and_then(|v| v.as_str()), Some("hold"));
        }
    }

    #[test]
    fn resolve_runtime_options_uses_defaults_when_missing() {
        let (mc, to, total_to) = resolve_runtime_options(None);
        assert_eq!(mc, DEFAULT_MAX_CONCURRENT);
        assert_eq!(to.as_secs(), DEFAULT_STEP_TIMEOUT_SECS);
        assert_eq!(total_to.as_secs(), DEFAULT_TOTAL_TIMEOUT_SECS);
    }

    #[test]
    fn resolve_runtime_options_reads_template_vars() {
        let vars = vec![
            Variable {
                name: "max_concurrent".into(),
                var_type: "number".into(),
                value: json!(20),
                description: None,
                is_secret: false,
            },
            Variable {
                name: "agent_timeout_secs".into(),
                var_type: "number".into(),
                value: json!(120),
                description: None,
                is_secret: false,
            },
        ];
        let (mc, to, _total_to) = resolve_runtime_options(Some(&vars));
        assert_eq!(mc, 20);
        assert_eq!(to.as_secs(), 120);
    }

    #[test]
    fn resolve_runtime_options_clamps_extremes() {
        let vars = vec![
            Variable {
                name: "max_concurrent".into(),
                var_type: "number".into(),
                value: json!(0), // 0 → clamp 到 1
                description: None,
                is_secret: false,
            },
            Variable {
                name: "agent_timeout_secs".into(),
                var_type: "number".into(),
                value: json!(99999), // 过大 → clamp 到 3600
                description: None,
                is_secret: false,
            },
        ];
        let (mc, to, _total_to) = resolve_runtime_options(Some(&vars));
        assert_eq!(mc, 1);
        assert_eq!(to.as_secs(), 3600);
    }

    #[test]
    fn resolve_runtime_options_falls_back_on_bad_types() {
        let vars = vec![Variable {
            name: "max_concurrent".into(),
            var_type: "string".into(),
            value: json!("not a number"),
            description: None,
            is_secret: false,
        }];
        let (mc, _to, _total_to) = resolve_runtime_options(Some(&vars));
        assert_eq!(mc, DEFAULT_MAX_CONCURRENT);
    }

    // ── extract_decision_json(修复"决策信息缺失"误报)──

    /// 优先取 results["portfolio-mgr"]["result"](CodeNode 包装内 Rhai 实际输出)
    #[test]
    pub(crate) fn extract_decision_json_prefers_portfolio_mgr_result() {
        use std::collections::HashMap;
        let mut results = HashMap::new();
        results.insert(
            "portfolio-mgr".to_string(),
            json!({
                "status": "executed",
                "language": "rhai",
                "result": {
                    "action": "买入",
                    "positionPct": 50.0,
                    "confidence": 75.0,
                    "riskLevel": "中",
                    "reasoning": "技术面强势",
                    "timeHorizon": "mid",
                    "expectedHoldingDays": 28,
                },
                "input_params": { "totalScore": 70.0 },
                "node_id": "portfolio-mgr",
                "params": { "action": "买入" },
            }),
        );
        // 即使 wf.output 存在且被 output_schema 污染成整个 results map,
        // 优先从 portfolio-mgr 节点本身提取。
        results.insert("trigger".to_string(), json!({ "status": "ok" }));
        let wf = Workflow {
            id: "test".to_string(),
            name: "test".to_string(),
            nodes: vec![],
            edges: vec![],
            status: axagent_rt_workflow::workflow_engine::WorkflowStatus::Completed,
            created_at: 0,
            completed_at: None,
            results,
            node_states: HashMap::new(),
            output: Some(json!({
                "trigger": { "status": "ok" },
                "portfolio-mgr": { "status": "executed", "result": { "action": "买入" } },
                "end-output": { "status": "ok" },
            })),
            hooks_config: None,
            error_config: None,
            error_workflow_id: None,
        };
        let dj = extract_decision_json(&wf).expect("必须返回决策 JSON");
        let parsed: serde_json::Value = serde_json::from_str(&dj).expect("必须可解析");
        // 关键:从 portfolio-mgr.result 提取,action 是 "买入" 而非被 output 污染
        assert_eq!(parsed["action"], "买入");
        assert_eq!(parsed["confidence"], 75.0);
        assert_eq!(parsed["positionPct"], 50.0);
        assert_eq!(parsed["riskLevel"], "中");
    }

    /// 2026-09-13 契约变更：`portfolio-risk-gate` 优先于 `portfolio-mgr`。
    ///
    /// 背景：模板四处（store-result.input_var / end-output.output_var / rule-check 与
    /// decision-explainer 的 contextSources）都声明风控门是链尾终值，但落库此前只认
    /// portfolio-mgr ⇒ 风控门的仓位修正（R-206）被整体丢弃，出现「报告说已清仓 0%、
    /// 界面说仍持 8.8%」的两套真相。本测试锁定「gate 存在且含 action ⇒ 取 gate」。
    #[test]
    pub(crate) fn extract_decision_json_prefers_portfolio_risk_gate_over_mgr() {
        use std::collections::HashMap;
        let mut results = HashMap::new();
        results.insert(
            "portfolio-mgr".to_string(),
            json!({
                "status": "executed",
                "result": {
                    "action": "减持",
                    "positionPct": 8.85,
                    "confidence": 44.7,
                    "riskLevel": "极高风险",
                    "reasoning": "公式原始决策",
                },
            }),
        );
        results.insert(
            "portfolio-risk-gate".to_string(),
            json!({
                "status": "executed",
                "result": {
                    "action": "减持",
                    "positionPct": 0.0,
                    "confidence": 44.7,
                    "riskLevel": "极高风险",
                    "reasoning": "公式原始决策 | [风控门] R-206 单股仓位 8.9% 超过上限，已下调",
                    "risk_gate": {
                        "adjusted": true,
                        "originalPositionPct": 8.85,
                        "reasons": ["R-206 单股仓位 8.9% 超过上限，已下调"],
                    },
                },
            }),
        );
        let wf = Workflow {
            id: "test".to_string(),
            name: "test".to_string(),
            nodes: vec![],
            edges: vec![],
            status: axagent_rt_workflow::workflow_engine::WorkflowStatus::Completed,
            created_at: 0,
            completed_at: None,
            results,
            node_states: HashMap::new(),
            output: None,
            hooks_config: None,
            error_config: None,
            error_workflow_id: None,
        };
        let dj = extract_decision_json(&wf).expect("必须返回决策 JSON");
        let parsed: serde_json::Value = serde_json::from_str(&dj).expect("必须可解析");
        assert_eq!(
            parsed["positionPct"], 0.0,
            "必须取风控门下调后的仓位，而非 portfolio-mgr 的 8.85"
        );
        assert_eq!(parsed["risk_gate"]["adjusted"], true, "风控门元数据必须随决策落库");
    }

    /// 风控门结果不可用（无 action）时回落到 portfolio-mgr，保持旧语义不被破坏。
    #[test]
    pub(crate) fn extract_decision_json_falls_back_to_mgr_when_gate_unusable() {
        use std::collections::HashMap;
        let mut results = HashMap::new();
        results.insert(
            "portfolio-mgr".to_string(),
            json!({
                "status": "executed",
                "result": { "action": "买入", "positionPct": 12.0, "confidence": 66.0 },
            }),
        );
        results.insert(
            "portfolio-risk-gate".to_string(),
            json!({ "status": "executed", "result": { "risk_gate": { "adjusted": false } } }),
        );
        let wf = Workflow {
            id: "test".to_string(),
            name: "test".to_string(),
            nodes: vec![],
            edges: vec![],
            status: axagent_rt_workflow::workflow_engine::WorkflowStatus::Completed,
            created_at: 0,
            completed_at: None,
            results,
            node_states: HashMap::new(),
            output: None,
            hooks_config: None,
            error_config: None,
            error_workflow_id: None,
        };
        let dj = extract_decision_json(&wf).expect("必须回落到 portfolio-mgr");
        let parsed: serde_json::Value = serde_json::from_str(&dj).expect("必须可解析");
        assert_eq!(parsed["action"], "买入");
        assert_eq!(parsed["positionPct"], 12.0);
    }

    /// 2026-09-13 顺序约束：D/F 档下 `quality-fallback` 必须压过 `portfolio-risk-gate`。
    ///
    /// 拓扑：portfolio-mgr → portfolio-risk-gate → rule-check → quality-gate。
    /// quality-gate 判 D/F 时（`default_case = "low-quality"`）路由到 quality-fallback，
    /// 由 LLM 保守决策**替代**公式决策。但风控门在 quality-gate **之前**就已产出结果
    /// ⇒ 若把 gate 放在最高优先级，就会拿「已被质量门替代的公式决策」当最终结论，
    /// 直接推翻 V40 修复。本测试用「gate 有 action 且 qf 有 action」同时在场的场景锁定顺序。
    #[test]
    pub(crate) fn extract_decision_json_prefers_quality_fallback_over_gate() {
        use std::collections::HashMap;
        let mut results = HashMap::new();
        results.insert(
            "portfolio-mgr".to_string(),
            json!({ "status": "executed", "result": { "action": "减持", "positionPct": 8.85 } }),
        );
        results.insert(
            "portfolio-risk-gate".to_string(),
            json!({
                "status": "executed",
                "result": { "action": "减持", "positionPct": 0.0, "reasoning": "风控门下调" },
            }),
        );
        results.insert(
            "quality-fallback".to_string(),
            json!({
                "content": r#"{"action":"观望","positionPct":0,"confidence":25,"riskLevel":"高风险","reasoning":"数据质量 D 级，保守观望"}"#,
            }),
        );
        let wf = Workflow {
            id: "test".to_string(),
            name: "test".to_string(),
            nodes: vec![],
            edges: vec![],
            status: axagent_rt_workflow::workflow_engine::WorkflowStatus::Completed,
            created_at: 0,
            completed_at: None,
            results,
            node_states: HashMap::new(),
            output: None,
            hooks_config: None,
            error_config: None,
            error_workflow_id: None,
        };
        let dj = extract_decision_json(&wf).expect("必须返回决策 JSON");
        let parsed: serde_json::Value = serde_json::from_str(&dj).expect("必须可解析");
        assert_eq!(parsed["action"], "观望", "D/F 档链尾是 quality-fallback，不是风控门");
        assert_eq!(parsed["riskLevel"], "高风险");
        // LLM 未输出 decisionConfidence 时由本函数补齐（P0 默认值语义）
        assert_eq!(parsed["decisionConfidence"], 25.0);
    }

    /// 2026-09-13 新增：`extract_formula_decision_json` 只认确定性节点，
    /// **绝不能**把 LLM 兜底当成「公式侧」。
    ///
    /// 实证 600031（2026-09-13）：`compute_decision_agreement` 此前直接用
    /// `extract_decision_json` 当公式侧，于是 `formulaAction="减持"`/
    /// `formulaRiskLevel="高风险"` 其实是 `quality-fallback` 的 **LLM** 输出，
    /// 而真正的公式决策是风控门的 `增持`/`中风险`（11.5%）—— 前端渲染成
    /// 「公式 ◀ 53 ▶ LLM」，用户看到的「公式」其实是 LLM（铁律 41）。
    #[test]
    pub(crate) fn formula_decision_json_never_returns_llm_fallback() {
        use std::collections::HashMap;
        let mut results = HashMap::new();
        results.insert(
            "portfolio-mgr".to_string(),
            json!({ "status": "executed", "result": { "action": "增持", "positionPct": 11.5 } }),
        );
        results.insert(
            "portfolio-risk-gate".to_string(),
            json!({
                "node_id": "end-output",
                "output": { "action": "增持", "positionPct": 11.5, "riskLevel": "中风险" },
                "status": "terminated",
            }),
        );
        results.insert(
            "quality-fallback".to_string(),
            json!({
                "content": r#"{"action":"减持","positionPct":5,"confidence":30,"riskLevel":"高风险"}"#,
            }),
        );
        let wf = Workflow {
            id: "test".to_string(),
            name: "test".to_string(),
            nodes: vec![],
            edges: vec![],
            status: axagent_rt_workflow::workflow_engine::WorkflowStatus::Completed,
            created_at: 0,
            completed_at: None,
            results,
            node_states: HashMap::new(),
            output: None,
            hooks_config: None,
            error_config: None,
            error_workflow_id: None,
        };
        // 公式侧：取风控门（穿透 EndNode 的 {node_id, output, status} 包装）
        let f = extract_formula_decision_json(&wf).expect("必须返回公式决策");
        let parsed: serde_json::Value = serde_json::from_str(&f).expect("必须可解析");
        assert_eq!(parsed["action"], "增持", "公式侧必须是 gate 的增持，不得是 LLM 兜底的减持");
        assert_eq!(parsed["positionPct"], 11.5);
        assert_eq!(parsed["riskLevel"], "中风险");
        // 对照：落库口径（链尾优先）仍取 quality-fallback —— 两者分工不同
        let stored: serde_json::Value =
            serde_json::from_str(&extract_decision_json(&wf).unwrap()).unwrap();
        assert_eq!(stored["action"], "减持", "落库口径仍取 D/F 档的链尾 LLM 保守决策");
    }

    /// 公式节点全部缺位时返回 `None`（不得退化成「随便拿一个」）。
    #[test]
    pub(crate) fn formula_decision_json_is_none_without_formula_nodes() {
        use std::collections::HashMap;
        let mut results = HashMap::new();
        results.insert(
            "quality-fallback".to_string(),
            json!({ "content": r#"{"action":"观望","positionPct":0}"# }),
        );
        let wf = Workflow {
            id: "test".to_string(),
            name: "test".to_string(),
            nodes: vec![],
            edges: vec![],
            status: axagent_rt_workflow::workflow_engine::WorkflowStatus::Completed,
            created_at: 0,
            completed_at: None,
            results,
            node_states: HashMap::new(),
            output: None,
            hooks_config: None,
            error_config: None,
            error_workflow_id: None,
        };
        assert!(
            extract_formula_decision_json(&wf).is_none(),
            "只有 LLM 兜底时必须返回 None，不能把它当公式决策"
        );
    }

    // ── extract_analyst_reports_from_snapshot（仪表盘分析师报告提取 + 双重编码穿透）──

    /// 双重编码：值是「JSON 字符串」且内部又是 `{"report": "<md>"}` → 必须穿透到纯 md 文本
    #[test]
    pub(crate) fn extract_analyst_reports_unwraps_double_encoded_report_json() {
        use std::collections::HashMap;
        let mut snapshot = HashMap::new();
        snapshot.insert(
            "report.a-hot-money".to_string(),
            json!({"report": "{\"report\": \"### 资金面报告\\n主力超大单承接不足，杠杆风险高\"}"}),
        );
        let reports = extract_analyst_reports_from_snapshot(&snapshot);
        let text = reports.get("hot-money-tracker").expect("必须提取到 hot-money-tracker 报告");
        assert!(text.starts_with("### 资金面报告"), "实际文本: {text}");
        assert!(!text.contains("{\"report\""), "不允许残留 JSON 包装: {text}");
        assert!(text.contains('\n'), "字面量 \\n 必须解为真实换行: {text}");
    }

    /// ToolNode 包装 `{content: "<json字符串>", tool_name}` → 穿透 content 后取 report 字段
    #[test]
    pub(crate) fn extract_analyst_reports_unwraps_tool_node_content_wrapper() {
        use std::collections::HashMap;
        let mut snapshot = HashMap::new();
        snapshot.insert(
            "a-news".to_string(),
            json!({
                "content": "{\"report\": \"公司发布业绩预增公告，属利好催化\"}",
                "tool_name": "agent_executor",
            }),
        );
        let reports = extract_analyst_reports_from_snapshot(&snapshot);
        let text = reports.get("news-analyst").expect("必须提取到 news-analyst 报告");
        assert_eq!(text, "公司发布业绩预增公告，属利好催化");
    }

    /// 围栏包裹 + 对象直出等形态均规整为纯文本
    #[test]
    pub(crate) fn extract_analyst_reports_strips_code_fence_and_keeps_plain_object() {
        use std::collections::HashMap;
        let mut snapshot = HashMap::new();
        snapshot.insert(
            "report.a-policy".to_string(),
            json!("```json\n{\"report\": \"产业政策利好落地\"}\n```"),
        );
        // 对象直出（历史路径原有形态，行为不能回退）
        snapshot.insert(
            "report.a-fundamentals".to_string(),
            json!({"report": "基本面稳健\n估值处于低位", "verdict": {"direction": "多"}}),
        );
        let reports = extract_analyst_reports_from_snapshot(&snapshot);
        assert_eq!(reports.get("policy-analyst").unwrap(), "产业政策利好落地");
        assert_eq!(reports.get("fundamentals-analyst").unwrap(), "基本面稳健\n估值处于低位");
    }

    /// 仪表盘风险描述：markdown 多行报告中应取到内容行而非整段表格/标题
    #[test]
    pub(crate) fn dashboard_risk_alert_description_extracts_content_line() {
        let mut analyst_reports = std::collections::HashMap::new();
        analyst_reports.insert(
            "hot-money-tracker".to_string(),
            "### 六、综合判断与风险\n\n| 维度 | 信号 | 方向 |\n|---|---|---|\n\
             | 主力超大单 | 单日 -3.26 亿，承接不足 | 空 |\n| 融资盘 | 16.58 亿，杠杆风险高 | 空 |\n\
             核心结论：短期资金面偏空。"
                .to_string(),
        );
        let alerts = axagent_analysis_engine::dashboard_report::__extract_risk_alerts_for_test(
            &analyst_reports,
        );
        assert_eq!(alerts.len(), 1);
        let desc = &alerts[0].description;
        assert_eq!(desc, "| 融资盘 | 16.58 亿，杠杆风险高 | 空 |");
    }

    /// V71 硬化契约：`portfolio-mgr` 节点 **Completed 但结果无可用 action**
    /// （CodeNode 包装里既无 `.result` 也无 `.output`，action 只藏在 `params` 里）⇒
    /// 必须落「决策缺失」占位决策，而**不是返回 None**（返回 None 会让 `decision_action`
    /// 落 NULL、前端显示"决策缺失"，正是 V71 要消灭的形态）。
    ///
    /// ⚠️ P0-5(2026-09-14): 占位 action 由「观望」改为显式缺失哨兵「数据缺失」。
    /// 原契约把「节点没产出决策」写成一个保守**结论**（观望），使它在 UI 与 DB 上
    /// 与真实观望无法区分 —— 前端据此渲染的操作标签是伪造的。
    ///
    /// ⚠️ 本测试原名 `extract_decision_json_falls_back_to_pm_wrapper_when_result_missing`，
    /// 断言的是 **V71 之前的契约**（无条件把包装本身当决策返回，断言 `parsed["params"]["action"]`）。
    /// V71 引入 `has_usable_action` 后该契约被取代，但测试未同步 ⇒ 长期红测。
    /// 佐证（无 git）：worktree `v297` / `pre992947df` 中**该测试存在而
    /// `has_usable_action` 尚不存在**（`grep -c` 分别为 1 / 0）—— 即失败早于本次改动。
    /// 2026-09-13 按现行契约重写，并补上「Completed + 无可辨识 action」这一原本未覆盖的分支
    /// （Failed / Skipped 分支已由 hardens_failed / hardens_skipped 两个测试覆盖）。
    #[test]
    pub(crate) fn extract_decision_json_hardens_completed_pm_without_action() {
        use std::collections::HashMap;
        let mut results = HashMap::new();
        results.insert(
            "portfolio-mgr".to_string(),
            json!({
                "status": "executed",
                "language": "rhai",
                // 故意无 .result / .output（异常路径）
                "params": { "action": "HOLD", "confidence": 30.0 },
                "node_id": "portfolio-mgr",
            }),
        );
        let mut node_states = HashMap::new();
        node_states.insert(
            "portfolio-mgr".to_string(),
            NodeRuntimeState {
                status: NodeStatus::Completed,
                attempts: 1,
                error: None,
                started_at: None,
                completed_at: None,
                skip_reason: None,
            },
        );
        let wf = Workflow {
            id: "test".to_string(),
            name: "test".to_string(),
            nodes: vec![],
            edges: vec![],
            status: axagent_rt_workflow::workflow_engine::WorkflowStatus::Completed,
            created_at: 0,
            completed_at: None,
            results,
            node_states,
            output: None,
            hooks_config: None,
            error_config: None,
            error_workflow_id: None,
        };
        let dj = extract_decision_json(&wf).expect("必须返回占位决策，而不是 None");
        let parsed: serde_json::Value = serde_json::from_str(&dj).expect("必须可解析");
        assert_eq!(parsed["action"], "数据缺失", "无可辨识 action ⇒ 显式缺失哨兵，不得伪装成观望");
        assert_eq!(parsed["positionPct"], 0.0);
        assert_eq!(parsed["confidence"], 0.0);
        assert_eq!(parsed["diagnostics"]["node"], "portfolio-mgr");
        assert_eq!(parsed["diagnostics"]["nodeStatus"], "Completed");
        // 归因必须指向「action 缺失」（形状里的键序依赖 serde_json Map 实现，故用前缀断言）
        assert!(
            parsed["diagnostics"]["errorCode"]
                .as_str()
                .unwrap_or_default()
                .starts_with("portfolio-mgr 结果缺少 action 字段"),
            "归因错误: {}",
            parsed["diagnostics"]["errorCode"]
        );
    }

    /// portfolio-mgr 节点不存在时回退到 wf.output(兼容无 portfolio-mgr 工作流)
    #[test]
    pub(crate) fn extract_decision_json_falls_back_to_workflow_output() {
        use std::collections::HashMap;
        let mut results = HashMap::new();
        results.insert("trigger".to_string(), json!({ "status": "ok" }));
        let wf = Workflow {
            id: "test".to_string(),
            name: "test".to_string(),
            nodes: vec![],
            edges: vec![],
            status: axagent_rt_workflow::workflow_engine::WorkflowStatus::Completed,
            created_at: 0,
            completed_at: None,
            results,
            node_states: HashMap::new(),
            output: Some(json!({ "action": "BUY", "confidence": 60.0 })),
            hooks_config: None,
            error_config: None,
            error_workflow_id: None,
        };
        let dj = extract_decision_json(&wf).expect("必须返回决策 JSON");
        let parsed: serde_json::Value = serde_json::from_str(&dj).expect("必须可解析");
        assert_eq!(parsed["action"], "BUY");
    }

    /// V57 硬化：portfolio-mgr 节点 Failed（Rhai 运行时错误）且 results 缺位时，
    /// 必须返回「数据缺失」占位决策并携带 diagnostics.nodeError，而非空壳。
    ///
    /// ⚠️ 断言在 P0-5(2026-09-14) 把占位 action 由「观望」改为「数据缺失」时**未同步**
    /// （同文件的 `..._hardens_completed_pm_without_action` 已断言「数据缺失」，
    /// 两条同语义测试自相矛盾）⇒ 长期红测。2026-09-14 A4 批次一并修正。
    #[test]
    pub(crate) fn extract_decision_json_hardens_failed_portfolio_mgr() {
        use std::collections::HashMap;
        let results = HashMap::new(); // 无 portfolio-mgr（节点失败未写入结果）
        let mut node_states = HashMap::new();
        node_states.insert(
            "portfolio-mgr".to_string(),
            NodeRuntimeState {
                status: NodeStatus::Failed,
                attempts: 1,
                error: Some("Rhai 执行失败: Variable not found: foo".to_string()),
                started_at: None,
                completed_at: None,
                skip_reason: None,
            },
        );
        let wf = Workflow {
            id: "test".to_string(),
            name: "test".to_string(),
            nodes: vec![],
            edges: vec![],
            status: axagent_rt_workflow::WorkflowStatus::Failed,
            created_at: 0,
            completed_at: None,
            results,
            node_states,
            output: None,
            hooks_config: None,
            error_config: None,
            error_workflow_id: None,
        };
        let dj = extract_decision_json(&wf).expect("失败节点也必须返回最小有效决策");
        let parsed: serde_json::Value = serde_json::from_str(&dj).expect("必须可解析");
        assert_eq!(
            parsed["action"], "数据缺失",
            "节点没产出决策 ⇒ 显式缺失哨兵，不得伪装成观望（P0-5 契约）"
        );
        assert_eq!(parsed["positionPct"], 0.0);
        assert_eq!(parsed["confidence"], 0.0);
        assert_eq!(parsed["diagnostics"]["node"], "portfolio-mgr");
        assert_eq!(parsed["diagnostics"]["nodeStatus"], "Failed");
        assert_eq!(parsed["diagnostics"]["nodeError"], "Rhai 执行失败: Variable not found: foo");
    }

    /// 资源键治理：节点错误里的 rt-workflow 错误码资源键（"{CODE}: {detail}"）
    /// 必须转为可读中文，且 reasoning 内联可读错误（不再出现"详见 diagnostics.nodeError"
    /// 指向一串资源键的死胡同）。
    #[test]
    pub(crate) fn humanize_node_error_translates_code_prefix() {
        use axagent_rt_workflow::work_engine::node_executor_trait::error_code as code;

        // 已知码前缀 → 可读中文 + 保留 detail 与原始码
        assert_eq!(
            humanize_node_error(&format!("{}: 东财 kline 接口超时", code::TIMEOUT)),
            "节点执行超时：东财 kline 接口超时（错误码 TIMEOUT）"
        );
        assert_eq!(
            humanize_node_error(&format!("{}: 所有供应商均失败", code::PROVIDER_QUERY_FAILED)),
            "数据供应商查询失败：所有供应商均失败（错误码 PROVIDER_QUERY_FAILED）"
        );

        // 引擎侧英文兜底文案
        assert_eq!(humanize_node_error("Node execution timeout"), "节点执行超时");
        assert_eq!(humanize_node_error("Circuit breaker open"), "熔断器开启（连续失败已暂停）");

        // 带降级标记的超时（engine 超时路径 "(degraded: skip)" 拼接）
        assert_eq!(
            humanize_node_error("Node execution timeout (degraded: skip)"),
            "节点执行超时 (degraded: skip)"
        );

        // 非"码前缀"形态（Rhai 错误等已可读）原样透传
        assert_eq!(
            humanize_node_error("Rhai 执行失败: Variable not found: foo"),
            "Rhai 执行失败: Variable not found: foo"
        );
        // 大小写不匹配的伪码不误翻
        assert_eq!(humanize_node_error("timeout: foo"), "timeout: foo");
    }

    /// V57 兜底 + 资源键治理联动：节点错误为错误码资源键形态时，
    /// nodeError 必须是可读中文、errorCode 保留原始串、reasoning 内联可读错误。
    #[test]
    pub(crate) fn extract_decision_json_hardened_node_error_is_humanized() {
        use axagent_rt_workflow::work_engine::node_executor_trait::error_code as code;
        use std::collections::HashMap;
        let results = HashMap::new();
        let mut node_states = HashMap::new();
        node_states.insert(
            "portfolio-mgr".to_string(),
            NodeRuntimeState {
                status: NodeStatus::Failed,
                attempts: 1,
                error: Some(format!("{}: 上游 LLM 无响应", code::LLM_CALL_FAILED)),
                started_at: None,
                completed_at: None,
                skip_reason: None,
            },
        );
        let wf = Workflow {
            id: "test".to_string(),
            name: "test".to_string(),
            nodes: vec![],
            edges: vec![],
            status: axagent_rt_workflow::WorkflowStatus::Failed,
            created_at: 0,
            completed_at: None,
            results,
            node_states,
            output: None,
            hooks_config: None,
            error_config: None,
            error_workflow_id: None,
        };
        let dj = extract_decision_json(&wf).expect("失败节点也必须返回最小有效决策");
        let parsed: serde_json::Value = serde_json::from_str(&dj).expect("必须可解析");
        assert_eq!(
            parsed["diagnostics"]["nodeError"],
            "LLM 调用失败：上游 LLM 无响应（错误码 LLM_CALL_FAILED）"
        );
        assert_eq!(
            parsed["diagnostics"]["errorCode"],
            format!("{}: 上游 LLM 无响应", code::LLM_CALL_FAILED)
        );
        let reasoning = parsed["reasoning"].as_str().unwrap();
        assert!(reasoning.contains("LLM 调用失败：上游 LLM 无响应"));
        assert!(!reasoning.contains("详见 diagnostics.nodeError"));
    }

    /// V57 硬化：portfolio-mgr 因上游失败被 Skipped 时同样兜底，
    /// 不再退化成 wf.output 空壳。
    ///
    /// A4(2026-09-14)：本测试同时守护新增的 `skip_reason` → `diagnostics.skippedNodes`
    /// 通路 —— 用户裁决「下游不该继续，但**应该告知错误**」，而此前的落库只剩
    /// 「上游依赖失败导致，无本地错误详情」这种零可行动信息的文案。
    #[test]
    pub(crate) fn extract_decision_json_hardens_skipped_portfolio_mgr() {
        use std::collections::HashMap;
        let results = HashMap::new();
        let mut node_states = HashMap::new();
        node_states.insert(
            "portfolio-mgr".to_string(),
            NodeRuntimeState {
                status: NodeStatus::Skipped,
                attempts: 0,
                error: None,
                started_at: None,
                completed_at: None,
                skip_reason: Some("upstream_failed".to_string()),
            },
        );
        let wf = Workflow {
            id: "test".to_string(),
            name: "test".to_string(),
            nodes: vec![],
            edges: vec![],
            status: axagent_rt_workflow::WorkflowStatus::PartiallyCompleted,
            created_at: 0,
            completed_at: None,
            results,
            node_states,
            output: None,
            hooks_config: None,
            error_config: None,
            error_workflow_id: None,
        };
        let dj = extract_decision_json(&wf).expect("跳过节点也必须返回最小有效决策");
        let parsed: serde_json::Value = serde_json::from_str(&dj).expect("必须可解析");
        assert_eq!(
            parsed["action"], "数据缺失",
            "被跳过的节点同样没产出决策 ⇒ 缺失哨兵（P0-5 契约）"
        );
        assert_eq!(parsed["diagnostics"]["nodeStatus"], "Skipped");
        // A4：跳过清单必须带出原因，且原因是引擎写入的分类值而非自由文本
        assert_eq!(parsed["diagnostics"]["skippedNodeCount"], 1);
        assert_eq!(parsed["diagnostics"]["skippedNodes"][0]["nodeId"], "portfolio-mgr");
        assert_eq!(parsed["diagnostics"]["skippedNodes"][0]["reason"], "upstream_failed");
        // 无 Failed 节点 ⇒ 不得凭空编造根因（宁可只说「原因：…」）
        assert!(parsed["diagnostics"]["rootFailureNode"].is_null());
        assert!(
            parsed["reasoning"].as_str().unwrap_or_default().contains("upstream_failed")
                || parsed["reasoning"].as_str().unwrap_or_default().contains("被级联跳过")
                || parsed["reasoning"].as_str().unwrap_or_default().contains("被跳过"),
            "reasoning 必须说明跳过事实: {}",
            parsed["reasoning"]
        );
    }

    /// P4 修复(2026-07-25): portfolio-mgr 节点在 results 和 node_states 中均缺位,
    /// 但 wf.output 是整个 results map(顶层含 trigger/portfolio-mgr 等 workflow
    /// 节点 ID)。旧逻辑直接序列化 wf.output,前端 normalizeDecision 识别为
    /// results map 后因 portfolio-mgr 缺失而判为"全零空壳"返回 null。
    ///
    /// 修复后:检测到 wf.output 是 results map 时,降级为最小占位决策
    /// (action="数据缺失" + diagnostics.nodeStatus="Missing"),前端不再误报"全零空壳"。
    /// （P0-5(2026-09-14) 起占位 action 由「观望」改为缺失哨兵；本断言于 2026-09-14
    ///  A4 批次同步，此前为红测。）
    #[test]
    pub(crate) fn extract_decision_json_hardens_workflow_output_results_map() {
        use std::collections::HashMap;
        let results = HashMap::new(); // results map 为空,模拟 portfolio-mgr 从未运行
        let node_states = HashMap::new(); // node_states 也无记录
        // wf.output 是整个 workflow 的 results map,顶层是节点 ID 而非决策字段
        let wf = Workflow {
            id: "test".to_string(),
            name: "test".to_string(),
            nodes: vec![],
            edges: vec![],
            status: axagent_rt_workflow::WorkflowStatus::Failed,
            created_at: 0,
            completed_at: None,
            results,
            node_states,
            output: Some(json!({
                "trigger": { "status": "Completed", "result": "ok" },
                "research-mgr": { "status": "Completed", "content": "..." },
                "trader": { "status": "Completed", "content": "{...}" },
                // 注意:portfolio-mgr 缺位(节点未运行)
            })),
            hooks_config: None,
            error_config: None,
            error_workflow_id: None,
        };
        let dj = extract_decision_json(&wf).expect("results map 必须降级为最小占位决策");
        let parsed: serde_json::Value = serde_json::from_str(&dj).expect("必须可解析");
        // 不应是 results map,应是最小占位决策
        assert_eq!(parsed["action"], "数据缺失");
        assert_eq!(parsed["positionPct"], 0.0);
        assert_eq!(parsed["confidence"], 0.0);
        assert_eq!(parsed["diagnostics"]["node"], "portfolio-mgr");
        assert_eq!(parsed["diagnostics"]["nodeStatus"], "Missing");
        // 确认没有把 trigger/research-mgr/trader 这些 results map key 写入
        assert!(parsed.get("trigger").is_none());
        assert!(parsed.get("research-mgr").is_none());
        assert!(parsed.get("trader").is_none());
    }

    /// A4（2026-09-14）：决策缺失时必须在 `diagnostics` 里带出**根因失败节点**。
    ///
    /// 场景取自 601166 实测拓扑（见 `AUDIT-601166-chain-break-2026-09-14.md`）：
    /// 容器体 `bear-r3`（上游 LLM 504）失败 → 其下游被引擎 fail-closed 级联标
    /// Skipped → `portfolio-mgr` 未产出决策。
    ///
    /// 旧行为只落一句「上游依赖失败导致，无本地错误详情」—— 真根因明明就躺在
    /// `node_states["bear-r3"].error` 里，却**零消费者**。本测试守护修复：
    /// ① 沿上游回溯到最远的 `Failed` 节点；② 跳过清单带原因；③ 用户可读文案
    /// 同时给出根因与影响面。
    #[test]
    pub(crate) fn extract_decision_json_reports_root_failure_and_skipped_nodes() {
        use axagent_harness::workflow_types::EdgeType;
        use std::collections::HashMap;
        let results = HashMap::new();
        let mut node_states = HashMap::new();
        node_states.insert(
            "bear-r3".to_string(),
            NodeRuntimeState {
                status: NodeStatus::Failed,
                attempts: 1,
                error: Some(
                    "UNSUPPORTED_PROVIDER: OpenAI API error 504: response headers not received within 15s"
                        .to_string(),
                ),
                started_at: None,
                completed_at: None,
                skip_reason: None,
            },
        );
        // 两个下游：p-risk-assess（中间跳）与 portfolio-mgr（链尾），均因上游失败被跳过
        for nid in ["p-risk-assess", "portfolio-mgr"] {
            node_states.insert(
                nid.to_string(),
                NodeRuntimeState {
                    status: NodeStatus::Skipped,
                    attempts: 0,
                    error: None,
                    started_at: None,
                    completed_at: None,
                    skip_reason: Some("upstream_failed".to_string()),
                },
            );
        }
        let mk_edge = |s: &str, t: &str| WorkflowEdge {
            id: format!("e-{s}-{t}"),
            source: s.to_string(),
            source_handle: None,
            target: t.to_string(),
            target_handle: None,
            edge_type: EdgeType::Direct,
            label: None,
        };
        let wf = Workflow {
            id: "test".to_string(),
            name: "test".to_string(),
            // nodes 留空：根因回溯只依赖 edges + node_states，节点标题缺失仅降级为空串
            nodes: vec![],
            edges: vec![
                mk_edge("p-risk-assess", "portfolio-mgr"),
                mk_edge("bear-r3", "p-risk-assess"),
            ],
            status: axagent_rt_workflow::WorkflowStatus::PartiallyCompleted,
            created_at: 0,
            completed_at: None,
            results,
            node_states,
            output: None,
            hooks_config: None,
            error_config: None,
            error_workflow_id: None,
        };
        let dj = extract_decision_json(&wf).expect("必须返回占位决策");
        let parsed: serde_json::Value = serde_json::from_str(&dj).expect("必须可解析");
        assert_eq!(parsed["action"], "数据缺失");
        // 根因必须回溯到**最远的 Failed 节点**（bear-r3），而非中间被跳过的 p-risk-assess
        assert_eq!(parsed["diagnostics"]["rootFailureNode"], "bear-r3");
        assert!(
            parsed["diagnostics"]["rootFailureError"].as_str().unwrap_or_default().contains("504"),
            "根因错误原文必须保留: {}",
            parsed["diagnostics"]["rootFailureError"]
        );
        // 跳过清单：2 项，原因均为引擎写入的分类值；按 nodeId 排序保证可复现
        assert_eq!(parsed["diagnostics"]["skippedNodeCount"], 2);
        assert_eq!(parsed["diagnostics"]["skippedNodes"][0]["nodeId"], "p-risk-assess");
        assert_eq!(parsed["diagnostics"]["skippedNodes"][1]["nodeId"], "portfolio-mgr");
        assert_eq!(parsed["diagnostics"]["skippedNodes"][1]["reason"], "upstream_failed");
        // 用户可读文案必须同时给出「根因是谁 / 错在哪 / 影响几个节点」
        let reasoning = parsed["reasoning"].as_str().unwrap_or_default();
        assert!(reasoning.contains("bear-r3"), "reasoning 缺根因节点: {reasoning}");
        assert!(reasoning.contains("504"), "reasoning 缺根因错误: {reasoning}");
        assert!(reasoning.contains("2 个下游节点"), "reasoning 缺影响面: {reasoning}");
    }
}

/// 从 `<!-- VERDICT: {...} -->` 标签中提取并解析 VERDICT JSON。
/// 旧版 snapshot 中数据质量报告（如 data-quality）被存储为
/// `"report文本<!-- VERDICT: {...} -->"` 格式的纯文本字符串，
/// 此函数从其中提取 VERDICT JSON 供后续字段导航恢复。
pub(crate) fn extract_verdict_from_text(text: &str) -> Option<serde_json::Value> {
    let start_marker = "<!-- VERDICT: ";
    let end_marker = "-->";
    if let Some(start) = text.rfind(start_marker) {
        let json_start = start + start_marker.len();
        if let Some(end_offset) = text[json_start..].find(end_marker) {
            let verdict_str = text[json_start..json_start + end_offset].trim();
            if !verdict_str.is_empty() {
                return serde_json::from_str::<serde_json::Value>(verdict_str).ok();
            }
        }
    }
    None
}

/// 仅重跑决策（portfolio-mgr CodeNode），不复用上游节点。
///
/// 从已有分析的 `blackboard_snapshot` 中读取缓存的所有上游节点输出，
/// 注入 portfolio-mgr 的 Rhai 脚本中重新计算决策。
/// 适用于：修改 portfolio-mgr.rhai 公式后快速验证，无需等待完整 DAG。
#[agent_command(domain = "finance", safety = Caution, call_mode = StateOnly, description =  "重运行股票决策计算")]
#[tauri::command]
pub async fn rerun_decision(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    analysis_id: String,
) -> Result<serde_json::Value, String> {
    use crate::commands::error::ErrorResponse;
    use rhai::Scope;
    use std::collections::HashMap;

    let db = state.harness.db();

    // 1. 加载分析记录
    let analysis = stock_analyses::Entity::find_by_id(&analysis_id)
        .one(db)
        .await
        .map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("查询分析记录失败: {e}"))
        })?
        .ok_or_else(|| format!("分析记录不存在: {analysis_id}"))?;

    // 2. 解析 blackboard_snapshot → variables map
    let snapshot_str = analysis.blackboard_snapshot.unwrap_or_default();
    let mut snapshot: HashMap<String, serde_json::Value> = serde_json::from_str(&snapshot_str)
        .map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL)
                .with_detail(format!("解析 blackboard_snapshot 失败: {e}"))
        })?;

    // 将 _raw.{nodeId} 条目提升到顶层（去除 _raw. 前缀），使 input_mapping
    // 中的原始 nodeId 路径（如 t-scoring.result.totalScore）能正确解析。
    // _raw.* 由 build_blackboard_snapshot 在 blackboard.rs 中写入。
    let raw_keys: Vec<String> =
        snapshot.keys().filter(|k| k.starts_with("_raw.")).cloned().collect();
    if !raw_keys.is_empty() {
        for raw_key in raw_keys {
            if let Some(key) = raw_key.strip_prefix("_raw.") {
                if let Some(val) = snapshot.remove(&raw_key) {
                    // 不覆盖已有 key（remapped key 优先）
                    snapshot.entry(key.to_string()).or_insert(val);
                }
            }
        }
    } else {
        // 旧版 snapshot（无 _raw.* 前缀）：反向推导 remapped key 的原始 nodeId
        let reverse_keys: Vec<(String, String)> = snapshot
            .keys()
            .filter_map(|k| {
                // ⚠️ 特定映射必须在通用 report.* 前缀匹配之前，
                // 否则 report.investment-plan 会被 strip_prefix("report.")
                // 截成 "investment-plan" 而非正确的 "trader"
                if *k == "report.investment-plan" {
                    Some(("trader".to_string(), k.clone()))
                } else if *k == "value.assessment" {
                    Some(("value-investor".to_string(), k.clone()))
                } else if *k == "rule_check.summary" {
                    Some(("rule-check".to_string(), k.clone()))
                } else if *k == "data_quality_summary" {
                    Some(("data-quality".to_string(), k.clone()))
                } else if *k == "raw.combined" {
                    Some(("raw-data".to_string(), k.clone()))
                } else {
                    k.strip_prefix("report.").map(|id| (id.to_string(), k.clone()))
                }
            })
            .collect();
        for (orig_id, remapped_key) in reverse_keys {
            if !snapshot.contains_key(&orig_id) {
                if let Some(val) = snapshot.get(&remapped_key) {
                    snapshot.insert(orig_id, val.clone());
                }
            }
        }
    }

    // 3. 加载工作流模板 → 提取 portfolio-mgr CodeNode
    let template = axagent_entities::workflow_template::Entity::find()
        .filter(axagent_entities::workflow_template::Column::Id.eq("stock-analysis"))
        .one(db)
        .await
        .map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("查询工作流模板失败: {e}"))
        })?
        .ok_or_else(|| "工作流模板不存在".to_string())?;

    let nodes: Vec<WorkflowNode> = serde_json::from_str(&template.nodes).map_err(|e| {
        ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("解析模板节点失败: {e}"))
    })?;

    // 找到 portfolio-mgr 节点及其 code + input_mapping
    let (code, input_mapping) = nodes
        .iter()
        .find_map(|n| {
            if let WorkflowNode::Code(cn) = n {
                if cn.config.execute_directly && cn.base.id == "portfolio-mgr" {
                    Some((cn.config.code.clone(), cn.config.input_mapping.clone()))
                } else {
                    None
                }
            } else {
                None
            }
        })
        .ok_or_else(|| "未找到 portfolio-mgr CodeNode".to_string())?;

    // 4. 执行 Rhai 脚本（复用 code_executor 的 register_common_functions，确保函数集一致）
    // 修复历史 bug：原手动注册漏掉 json_parse，导致 portfolio-mgr.rhai 的 safe_parse 在
    // rerun 路径下失败，进而使公告关键词检测 f3、资金面 f9、筹码面 f10、龙虎榜 f10、
    // PACE f11 等依赖 safe_parse 的下游逻辑全部失去数据。
    // 沙箱档位与宿主函数集统一走 rhai_registry::build_stock_rhai_engine：
    // 与共享 Engine 同源（common + pm_* + bottleneck_*），不再在此处手工注册。
    // 历史：本处只注册了 common + pm_*（缺 bottleneck_*），而
    // commands/stock_analysis.rs 的 What-If 入口更少（仅 common）
    // ⇒「同一脚本在不同入口能跑 / 不能跑」。2026-09-22 收敛为单一构造点。
    // 档位取 PORTFOLIO（256，实测下限 48 的 ~5 倍余量）：portfolio-mgr.rhai
    // 表达式嵌套深，默认上限会在**编译期**抛 "Expression exceeds maximum
    // complexity"，脚本内 try/catch 无法捕获，会导致整个节点无输出。
    let engine = super::rhai_registry::build_stock_rhai_engine(
        super::rhai_registry::RhaiSandboxLimits::PORTFOLIO,
    );
    let mut scope = Scope::new();

    // ── Gap 2: 注入近期 lessons（reflection_lessons 活跃规则）──
    // 让 portfolio-mgr.rhai 在决策时知道最近哪些股票犯过错。
    // lessons 按 confidence 降序取前 10 条，打包为 JSON 注入。
    {
        use axagent_entities::reflection_lessons;
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
        let recent_lessons: Vec<String> = reflection_lessons::Entity::find()
            .filter(reflection_lessons::Column::Status.eq("active"))
            .filter(reflection_lessons::Column::Confidence.gt(0.5))
            .order_by(reflection_lessons::Column::Confidence, sea_orm::Order::Desc)
            .limit(10)
            .all(db)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|l| l.lesson_summary)
            .collect();
        let lessons_json = serde_json::to_string(&recent_lessons).unwrap_or_else(|_| "[]".into());
        scope.push_constant("recent_lessons", lessons_json);
        tracing::debug!("[rerun_decision] Gap2: 注入 {} 条 lessons 到 scope", recent_lessons.len());
    }

    // 简化版 resolve_var_path：导航 JSON 嵌套（支持 JSON 字符串自动解析）
    fn resolve_path(
        path: &str,
        vars: &HashMap<String, serde_json::Value>,
    ) -> Option<serde_json::Value> {
        if path.is_empty() {
            return None;
        }
        let parts: Vec<&str> = path.split('.').collect();
        if let Some(root) = vars.get(parts[0]) {
            let mut current = root.clone();
            for part in &parts[1..] {
                if let serde_json::Value::String(s) = &current {
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(s) {
                        current = parsed;
                    }
                }
                current = current.get(part)?.clone();
            }
            Some(current)
        } else {
            vars.get(path).cloned()
        }
    }

    // 注入 input_mapping 到 Rhai scope
    let has_raw = snapshot.keys().any(|k| k.starts_with("_raw."));
    // V37: 旧版 snapshot（无 _raw.*）中 ToolNode/AgentNode 的值已被 extract_node_text
    // 提取为纯文本，JSON 结构已丢失，resolve_path 无法下钻到内部字段。
    // 剥除 .result./.content. 前缀后，子字段导航仍会失败（纯文本不是 JSON）。
    // 此时大部分 input_mapping 解析为 None，Rhai 侧 weights_collapsed 兜底。
    // 建议用户重新运行完整工作流以生成新版 snapshot。
    if !has_raw {
        tracing::warn!(
            "[rerun_decision] 旧版 snapshot（无 _raw.*），JSON 结构已丢失，建议重新运行完整工作流。input_mapping 将尽力使用已有数据。"
        );
    }

    // V40 修复: 旧版 snapshot 的 remapped key → 原始 nodeId 反向映射
    // build_blackboard_snapshot 对某些节点做了 key 重命名，此处构建反向表
    // 以便 resolve_path 能找到正确的键。
    let remap_old: std::collections::HashMap<&str, &str> = [
        ("data_quality_summary", "data-quality"),
        ("report.investment-plan", "trader"),
        ("value.assessment", "value-investor"),
        ("rule_check.summary", "rule-check"),
        ("raw.combined", "raw-data"),
    ]
    .into_iter()
    .collect();

    for (target_key, source_key) in &input_mapping {
        // 对于旧版 snapshot（无 _raw.*），尝试剥除 result./content. 前缀：
        // 因为旧版 build_blackboard_snapshot 已经把 ToolNode 的 result 和 AgentNode
        // 的 content 提取为纯文本，外层包裹已丢失。剥除后路径直接从 JSON 内容开始。
        let mut used_key = if has_raw {
            source_key.clone()
        } else {
            // 尝试剥除 node_id.result. → node_id. 和 node_id.content. → node_id.
            source_key.replacen(".result.", ".", 1).replacen(".content.", ".", 1)
        };
        // V40 修复: 旧版 snapshot 中 remapped key 的查找
        // resolve_path 的第一步是 vars.get(parts[0])，如果 parts[0] 是
        // "data-quality" 但旧版 snapshot 的 key 是 "data_quality_summary"，
        // 查找会失败。此处尝试用 remap_old 转换 key。
        if !has_raw {
            let first_seg = used_key.split('.').next().unwrap_or("");
            if let Some(&mapped) = remap_old.get(first_seg) {
                used_key = used_key.replacen(first_seg, mapped, 1);
            }
        }
        let value = resolve_path(&used_key, &snapshot);
        match &value {
            None | Some(serde_json::Value::Null) => {
                // V40: 旧版 snapshot 中值可能是纯文本字符串（extract_node_text），
                // 此时 resolve_path 找不到子字段（如 .content.score），但整条记录
                // 可能以字符串形式存在。尝试以 used_key 的 root 部分直查整个值。
                if !has_raw {
                    let root = used_key.split('.').next().unwrap_or("");
                    if let Some(full_text) = snapshot.get(root).and_then(|v| v.as_str()) {
                        let trimmed_text = full_text.trim().to_string();

                        // V42 增强: 旧版 snapshot 的文本中可能包含
                        // <!-- VERDICT: {...} --> 标签。尝试提取标签内的 JSON 并
                        // 按 used_key 中的子字段路径导航，以恢复结构化数据。
                        let mut injected_from_verdict = false;
                        if let Some(verdict_json) = extract_verdict_from_text(&trimmed_text) {
                            // 从 used_key 中提取子字段路径（去掉 root 部分）
                            let used_parts: Vec<&str> = used_key.split('.').collect();
                            if used_parts.len() > 1 {
                                let mut cur = &verdict_json;
                                for part in &used_parts[1..] {
                                    cur = match cur.get(*part) {
                                        Some(v) => v,
                                        None => {
                                            cur = &serde_json::Value::Null;
                                            break;
                                        },
                                    };
                                }
                                if !cur.is_null() {
                                    match cur {
                                        serde_json::Value::Number(n) => {
                                            let val = n.as_f64().unwrap_or(0.0);
                                            let _ = scope.push_constant(target_key.as_str(), val);
                                            tracing::info!(
                                                "[rerun_decision] 旧版 snapshot VERDICT 恢复: {target_key} ← {root}<!--VERDICT-->#{part} = {val}",
                                                part = used_parts[1..].join(".")
                                            );
                                            injected_from_verdict = true;
                                        },
                                        serde_json::Value::String(s) => {
                                            let _ =
                                                scope.push_constant(target_key.as_str(), s.clone());
                                            tracing::info!(
                                                "[rerun_decision] 旧版 snapshot VERDICT 恢复: {target_key} ← {root}<!--VERDICT-->#{part} = {s}",
                                                part = used_parts[1..].join(".")
                                            );
                                            injected_from_verdict = true;
                                        },
                                        _ => {},
                                    }
                                }
                            }
                        }

                        if injected_from_verdict {
                            continue;
                        }

                        // 尝试解析为数字（如 "B" 等级文本虽然无法解析，但 score 字段
                        // 如 "85" 可以解析为数字）
                        if let Ok(num) = trimmed_text.parse::<f64>() {
                            let _ = scope.push_constant(target_key.as_str(), num);
                            tracing::warn!(
                                "[rerun_decision] 旧版 snapshot 回退: {target_key} ← {root} (解析为数字 {num})"
                            );
                        } else {
                            // V40: 纯文本字符串不能注入给预期为数字的 Rhai 变量
                            //（如 dqi_score 若为文本会导致 (dqi_score-50)/50 类型错误）。
                            // 只对已知文本字段注入字符串，其余推入 () 让
                            // Rhai 侧走 weights_collapsed 兜底。
                            if target_key == "stock_lessons" || target_key == "sanity_reason" {
                                let _ = scope.push_constant(target_key.as_str(), trimmed_text);
                                tracing::warn!(
                                    "[rerun_decision] 旧版 snapshot 回退: {target_key} ← {root} (纯文本)"
                                );
                            } else {
                                let _ = scope.push_constant(target_key.as_str(), ());
                                tracing::warn!(
                                    "[rerun_decision] 旧版 snapshot 回退: {target_key} ← {root} (纯文本无法用于数值计算，放弃)"
                                );
                            }
                        }
                        continue;
                    }
                }
                let _ = scope.push_constant(target_key.as_str(), ());
            },
            Some(serde_json::Value::Number(n)) => {
                let val = n.as_f64().unwrap_or(0.0);
                let _ = scope.push_constant(target_key.as_str(), val);
            },
            Some(serde_json::Value::String(s)) => {
                let _ = scope.push_constant(target_key.as_str(), s.clone());
            },
            Some(serde_json::Value::Bool(b)) => {
                let _ = scope.push_constant(target_key.as_str(), *b);
            },
            Some(serde_json::Value::Array(arr)) => {
                let items: rhai::Array = arr
                    .iter()
                    .map(|v| match v {
                        serde_json::Value::Number(n) => {
                            rhai::Dynamic::from(n.as_f64().unwrap_or(0.0))
                        },
                        serde_json::Value::String(s) => rhai::Dynamic::from(s.clone()),
                        serde_json::Value::Bool(b) => rhai::Dynamic::from(*b),
                        _ => rhai::Dynamic::UNIT,
                    })
                    .collect();
                scope.push_dynamic(target_key.as_str(), rhai::Dynamic::from(items));
            },
            Some(serde_json::Value::Object(obj)) => {
                let mut map = rhai::Map::new();
                for (k, v) in obj {
                    let val = match v {
                        serde_json::Value::Number(n) => {
                            rhai::Dynamic::from(n.as_f64().unwrap_or(0.0))
                        },
                        serde_json::Value::String(s) => rhai::Dynamic::from(s.clone()),
                        serde_json::Value::Bool(b) => rhai::Dynamic::from(*b),
                        _ => continue,
                    };
                    map.insert(k.clone().into(), val);
                }
                scope.push_dynamic(target_key.as_str(), rhai::Dynamic::from(map));
            },
        }
    }

    // 执行 Rhai 脚本
    // P1-D10: 通过全局 AST 缓存复用编译结果，避免 Rerun Decision 时重复编译。
    // 注意：code 来自数据库 workflow_template，可能与 include_str! 版本不同
    // （用户修改了 portfolio-mgr.rhai 后重新 seed），AST 缓存按 code hash 区分，
    // code 变化时自动产生新 key，不会命中旧缓存。
    let ast = axagent_harness::get_or_compile_ast("portfolio-mgr-rerun", &code, &engine).map_err(
        |e| ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("Rhai AST 编译失败: {e}")),
    )?;

    let result: rhai::Dynamic = engine.eval_ast_with_scope(&mut scope, &ast).map_err(|e| {
        ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("Rhai 脚本执行失败: {e}"))
    })?;

    // 转换 Rhai 结果到 JSON
    fn to_json(v: &rhai::Dynamic) -> serde_json::Value {
        if v.is_unit() {
            return serde_json::Value::Null;
        }
        if v.is_bool() {
            return serde_json::Value::Bool(v.as_bool().unwrap_or(false));
        }
        if let Ok(s) = v.clone().into_string() {
            return serde_json::Value::String(s);
        }
        if let Ok(f) = v.as_float() {
            if let Some(n) = serde_json::Number::from_f64(f) {
                return serde_json::Value::Number(n);
            }
        }
        if let Some(arr) = v.clone().try_cast::<rhai::Array>() {
            return serde_json::Value::Array(arr.into_iter().map(|item| to_json(&item)).collect());
        }
        if let Some(map) = v.clone().try_cast::<rhai::Map>() {
            let mut obj = serde_json::Map::new();
            for (k, val) in &map {
                obj.insert(format!("{k}"), to_json(val));
            }
            return serde_json::Value::Object(obj);
        }
        serde_json::Value::String(format!("{v}"))
    }
    let decision_value = to_json(&result);

    // 5. 提取决策字段
    let action = decision_value.get("action").and_then(|v| v.as_str()).map(|s| s.to_string());
    let position_pct = decision_value.get("positionPct").and_then(|v| v.as_f64());
    let confidence = decision_value.get("confidence").and_then(|v| v.as_f64());
    let reasoning = decision_value.get("reasoning").and_then(|v| v.as_str()).map(|s| s.to_string());
    let time_horizon =
        decision_value.get("timeHorizon").and_then(|v| v.as_str()).map(|s| s.to_string());
    let holding_days = decision_value.get("expectedHoldingDays").and_then(|v| {
        if let Some(f) = v.as_f64() {
            Some(f as i64)
        } else {
            v.as_i64()
        }
    });

    // 跨系统互证（crossCheck）：与近 14 天智选推荐对照（与 run_stock_workflow
    // 持久化路径同语义，重跑决策后互证字段不丢失）
    let mut decision_value = decision_value;
    if let Some(prior) = super::hooks::fetch_reco_prior(db, &analysis.stock_code, 14).await {
        super::hooks::inject_reco_crosscheck(&mut decision_value, &prior);
    }

    let decision_json_str = serde_json::to_string(&decision_value).unwrap_or_default();

    // 6. 更新分析记录
    stock_analyses::Entity::update_many()
        .col_expr(stock_analyses::Column::DecisionAction, Expr::value(action))
        .col_expr(stock_analyses::Column::DecisionPositionPct, Expr::value(position_pct))
        .col_expr(stock_analyses::Column::DecisionReasoning, Expr::value(reasoning))
        .col_expr(stock_analyses::Column::DecisionJson, Expr::value(decision_json_str))
        // A4（决策可离线复算）：本函数第 3 步是**从 DB 现读** `stock-analysis` 模板，
        // 用途就是「改完 portfolio-mgr.rhai 拿旧快照复算验证」 ⇒ 本次产出的决策
        // 属于**当前**模板版本，必须覆盖旧值，否则该行的 `template_version` 会一直
        // 指向上一轮那个更旧的公式版本。
        //
        // ⚠ 注意 `blackboard_snapshot` **不被本函数重写**（快照仍是旧轮采集的）。
        // 于是「快照版本 ≠ 决策版本」是这条路径的**正常状态**，不是脏数据：
        //   template_version = 产出 DecisionJson 的公式版本（本次模板）
        //   快照自身的采集轮次     = 由 blackboard_snapshot 内容决定
        // 复算器必须能区分这两者，才不至于把「用新公式复算旧输入」误判成数据不一致。
        .col_expr(stock_analyses::Column::TemplateVersion, Expr::value(template.version))
        .col_expr(stock_analyses::Column::DecisionTimeHorizon, Expr::value(time_horizon))
        .col_expr(stock_analyses::Column::DecisionExpectedHoldingDays, Expr::value(holding_days))
        .col_expr(
            stock_analyses::Column::UpdatedAt,
            Expr::value(chrono::Utc::now().timestamp_millis()),
        )
        .filter(stock_analyses::Column::Id.eq(&analysis_id))
        .exec(db)
        .await
        .map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("更新分析记录失败: {e}"))
        })?;

    tracing::warn!(
        "[rerun_decision] 决策重跑完成: analysis_id={analysis_id}, confidence={confidence:?}"
    );

    // 7. 构建 DashboardReport（借鉴 daily_stock_analysis 决策仪表盘格式）
    // 从 snapshot 提取评分节点输出和专家报告（穿透 ToolNode content 包装）
    let score_json = extract_score_json(&snapshot);

    let analyst_reports = extract_analyst_reports_from_snapshot(&snapshot);
    let stock_code = analysis.stock_code.clone();
    let stock_name = analysis.stock_name.clone();
    let analysis_date = analysis.analysis_date.clone();

    let dashboard_value =
        merge_price_fields_from_llm(&decision_value, analysis.llm_decision_json.as_deref());
    let dashboard_report =
        axagent_analysis_engine::dashboard_report::build_dashboard_report_from_workflow(
            &dashboard_value,
            &score_json,
            &stock_code,
            &stock_name,
            &analysis_date,
            &analyst_reports,
            extract_valuation_json(&snapshot).as_ref(),
        );
    let dashboard_md =
        axagent_analysis_engine::dashboard_report::render_dashboard_md(&dashboard_report);

    tracing::info!(
        "[rerun_decision] DashboardReport 生成完成: integrity_passed={}, risk_alerts={}, catalysts={}",
        dashboard_report.integrity_passed,
        dashboard_report.risk_alerts.len(),
        dashboard_report.catalysts.len()
    );

    // 8. 重跑「仿真验证」（v45）
    //
    // 为什么必须在这里重跑：本函数会改 action / positionPct，但**不重写
    // blackboard_snapshot** ⇒ 快照里的 `sim-verify` 仍是**上一版决策**的结果。
    // 前端若继续展示它，就成了「决策已变、补充信息未变」的静默误导。
    // 与第 6 步清空 `decisionExplanation` 是同一类问题的同一种处理。
    //
    // 参考价传 `None`：本入口没有新的行情输入，重算用的就是快照里那份数据，
    // 故由挂钩从快照的 `t-scoring.result.content.currentPrice` 取价（同口径）。
    // 挂钩内部 `tokio::spawn`，不阻塞本次返回；结果经 `simulation-ready`
    // 事件推给已打开的分析页。
    super::sim_hook::spawn_simulation_after_decision(
        db.clone(),
        app,
        analysis_id.clone(),
        stock_code.clone(),
        None,
    );

    // A4（决策可离线复算）：把「原决策版本 vs 本次复算版本」的漂移显式回报。
    //
    // 为什么必须有这个**读数端**：`stock_analyses.template_version` 如果只写不读，
    //   就是一个没人消费的字段 —— 而它存在的唯一理由，是回答「这条历史决策是用
    //   哪版公式算出来的」。不回答这个问题的复算，正是 09-12 踩过的坑：同一批样本
    //   按新版公式复算后 posterior **系统性偏高 ~4.5pt**
    //   （`portfolio-mgr.rhai:2146-2147` 记载，12/13 样本跑在 v10~v26），
    //   而当时没有任何运行时信号提示「你正在跨版本比较」。
    //
    // `recorded` 取自本次更新**之前**的行值：`analysis` 是第 1 步读出的旧模型，
    //   第 6 步的 `update_many` 不会回写它 ⇒ 它忠实地是「原决策落库时的版本」。
    //   为 `None` 属**正常形态**：A4 之前的存量行、以及 chat 通道（`hooks.rs`）
    //   写入的行本就没有版本信息 —— 不要把它当脏数据修。
    let recorded_version = analysis.template_version;
    let current_version = template.version;
    let version_drifted = recorded_version.is_some_and(|v| v != current_version);
    let template_version_info = json!({
        "recorded": recorded_version,
        "current": current_version,
        // drifted = true ⇒ 本次复算用了与原决策**不同**的公式版本。此时输出与原决策
        //   的差异**不能**归因为「快照/数据不一致」，必须先排除公式变更。
        "drifted": version_drifted,
        "note": if version_drifted {
            "本次复算所用模板版本与原决策不同：差异可能来自公式变更，而非输入变化"
        } else if recorded_version.is_none() {
            "原决策未记录模板版本（A4 之前的存量行或 chat 通道写入），无法判定是否跨版本"
        } else {
            "同版本复算"
        },
    });

    Ok(json!({
        "analysis_id": analysis_id,
        "decision": decision_value,
        "llm_decision_json": analysis.llm_decision_json,
        "dashboardReport": dashboard_report,
        "dashboardMd": dashboard_md,
        "templateVersion": template_version_info,
    }))
}

/// 从 blackboard snapshot 提取分析师报告文本
///
/// snapshot 中的 key 格式为 `report.{expert_id}`（如 `report.a-fundamentals`），
/// 值为 JSON 字符串或对象。本函数把 `a-` 前缀去掉，映射到不带前缀的 expert_id
/// （如 `fundamentals-analyst`），供 `build_dashboard_report_from_workflow` 使用。
pub(crate) fn extract_analyst_reports_from_snapshot(
    snapshot: &std::collections::HashMap<String, serde_json::Value>,
) -> std::collections::HashMap<String, String> {
    let mut reports = std::collections::HashMap::new();

    // 专家 ID 映射：snapshot key 前缀 → build_dashboard_report_from_workflow 期望的 key
    let expert_mapping: &[(&str, &str)] = &[
        ("a-market-analyst", "market-analyst"),
        ("a-sentiment", "sentiment-analyst"),
        ("a-news", "news-analyst"),
        ("a-fundamentals", "fundamentals-analyst"),
        ("a-policy", "policy-analyst"),
        ("a-hot-money", "hot-money-tracker"),
        ("a-lockup", "lockup-watcher"),
    ];

    for (node_id, target_id) in expert_mapping {
        // 尝试两种 key 格式：report.{node_id} 和 {node_id}
        let report_key = format!("report.{node_id}");
        let value = snapshot.get(&report_key).or_else(|| snapshot.get(*node_id));
        if let Some(val) = value {
            // 深度穿透 ToolNode/AgentNode 包装（content/result/字符串二次编码），
            // 与 extract_score_json 同源；否则实时路径拿到 `{"report":"..."}` 双重编码
            // 原始串，仪表盘风险警报/催化因素显示整段转义文本（2026-09-11 光库实证）
            let unwrapped = unwrap_tool_node_content(val);
            let text = unwrap_report_text(&unwrapped);
            if !text.is_empty() {
                reports.insert((*target_id).to_string(), text);
            }
        }
    }

    reports
}

/// 把节点输出值规整为纯报告文本（最多 3 轮，防失控）。
///
/// 处理形态：对象 {report|content|text: "<文本>"}；JSON 字符串二次编码
/// （字符串本身可 parse）；markdown 代码围栏包裹。规整失败则回退
/// Value 的字符串表示（与旧行为一致，不丢数据）。
fn unwrap_report_text(val: &serde_json::Value) -> String {
    let mut cur = val.clone();
    for _ in 0..4 {
        cur = match cur {
            serde_json::Value::String(ref s) => {
                let trimmed = strip_md_code_fence(s.trim());
                match serde_json::from_str::<serde_json::Value>(&trimmed) {
                    // 字符串二次编码：解一层转义（\n 字面量 → 真实换行）
                    Ok(serde_json::Value::String(inner)) => {
                        let inner = inner.trim().to_string();
                        if inner == trimmed {
                            return inner;
                        }
                        serde_json::Value::String(inner)
                    },
                    Ok(parsed @ serde_json::Value::Object(_)) => parsed,
                    _ => return trimmed,
                }
            },
            serde_json::Value::Object(ref obj) => {
                let picked = obj
                    .get("report")
                    .or_else(|| obj.get("content"))
                    .or_else(|| obj.get("text"))
                    .cloned();
                match picked {
                    Some(v) if !v.is_null() => v,
                    _ => return cur.to_string(),
                }
            },
            _ => return cur.to_string(),
        };
    }
    // 预算耗尽：String 直接还原文本，其余回退 JSON 表示（与旧行为一致，不丢数据）
    match cur {
        serde_json::Value::String(s) => s,
        other => other.to_string(),
    }
}

/// 剥掉 markdown 代码围栏（``` 包裹），返回内部文本。
fn strip_md_code_fence(s: &str) -> String {
    let trimmed = s.trim();
    if let Some(rest) = trimmed.strip_prefix("```") {
        // 只剥已知语言标签，避免误删正文首字符
        let rest = rest
            .strip_prefix("json")
            .or_else(|| rest.strip_prefix("md"))
            .or_else(|| rest.strip_prefix("markdown"))
            .or_else(|| rest.strip_prefix("text"))
            .unwrap_or(rest);
        let rest = rest.trim_start_matches('\n');
        let body = match rest.rfind("```") {
            Some(end) => &rest[..end],
            None => rest,
        };
        return body.trim().to_string();
    }
    trimmed.to_string()
}

/// 从 Workflow 执行结果构建 DashboardReport + Markdown 文本。
///
/// 复用 rerun_decision 中的构建逻辑，让正常完成的 run_stock_workflow
/// 也能在 workflow-completed 事件中携带 dashboard 数据，避免前端
/// dashboardReport 在工作流完成后仍为 null（概览/仪表板 tab 永远显示空态）。
///
/// 内部使用 extract_decision_json 提取 portfolio-mgr 决策，
/// 从 results["t-scoring"] 提取评分 JSON，从 results 提取分析师报告。
pub(crate) fn build_dashboard_from_workflow_result(
    wf: &Workflow,
    stock_code: &str,
    stock_name: &str,
    analysis_date: &str,
) -> Option<(axagent_harness::DashboardReport, String)> {
    // 1. 提取决策 JSON 字符串并 parse 为 Value
    let decision_str = extract_decision_json(wf)?;
    let decision_value: serde_json::Value = serde_json::from_str(&decision_str).unwrap_or(
        serde_json::json!({"action": "观望", "positionPct": 0, "confidence": 0.0, "reasoning": ""}),
    );

    // 2. 提取评分 JSON（穿透 ToolNode content 包装，取真实 {total, signal, ...}）
    let score_json = extract_score_json(&wf.results);

    // 3. 提取分析师报告
    let analyst_reports = extract_analyst_reports_from_snapshot(&wf.results);

    // 4. 构建 DashboardReport（目标价/止损价缺键时从 trader LLM 决策兜底）
    let dashboard_value =
        merge_price_fields_from_llm(&decision_value, extract_llm_decision_json(wf).as_deref());
    let dashboard_report =
        axagent_analysis_engine::dashboard_report::build_dashboard_report_from_workflow(
            &dashboard_value,
            &score_json,
            stock_code,
            stock_name,
            analysis_date,
            &analyst_reports,
            extract_valuation_json(&wf.results).as_ref(),
        );
    let dashboard_md =
        axagent_analysis_engine::dashboard_report::render_dashboard_md(&dashboard_report);

    tracing::info!(
        "[build_dashboard_from_workflow_result] DashboardReport 构建完成: \
         integrity_passed={}, risk_alerts={}, catalysts={}",
        dashboard_report.integrity_passed,
        dashboard_report.risk_alerts.len(),
        dashboard_report.catalysts.len()
    );

    Some((dashboard_report, dashboard_md))
}

/// 节点输出深度穿透：剥掉评分类节点输出的多层包装，直到拿到业务 JSON。
///
/// 600089 实测的三种真实包装形态（评分 JSON 最深被包了三层）：
/// - 顶层 `t-scoring`：**字符串** `{"node_id","result":{"content":"<评分JSON字符串>"}}`
/// - `_raw.t-scoring`：`{node_id, result: {content: "<评分JSON字符串>"}, tool_name}`
/// - `result.t-scoring`：`{content: "<评分JSON字符串>"}`
///
/// 剥壳规则（最多 6 层防失控）：字符串可 parse → parse 继续；对象含 `content` →
/// 下钻 content；否则对象含 `result` → 下钻 result；拿到既无 content 也无 result
/// 的对象（如 {total, signal, ...}）即为目标。
pub(crate) fn unwrap_tool_node_content(val: &serde_json::Value) -> serde_json::Value {
    let mut cur = val.clone();
    for _ in 0..6 {
        cur = match cur {
            serde_json::Value::String(ref s) => {
                match serde_json::from_str::<serde_json::Value>(s) {
                    Ok(parsed) => parsed,
                    Err(_) => return cur,
                }
            },
            serde_json::Value::Object(ref obj) => {
                if let Some(content) = obj.get("content") {
                    content.clone()
                } else if let Some(result) = obj.get("result") {
                    result.clone()
                } else {
                    return cur;
                }
            },
            _ => return cur,
        };
    }
    cur
}

/// 从 snapshot / workflow results map 中提取 t-scoring 评分 JSON。
///
/// 兼容三种 key（remapped / _raw 提升 / 旧版 `.result` 后缀）并穿透 ToolNode 的
/// `{content, tool_name}` 包装。此前三处 dashboard 构建直接在包装层上 `.get("total")`，
/// 恒为 Null → 评分恒 0/100、趋势恒「震荡」（2026-09-11 科华数据实证）。
pub(crate) fn extract_score_json(
    map: &std::collections::HashMap<String, serde_json::Value>,
) -> serde_json::Value {
    let raw = map
        .get("t-scoring")
        .or_else(|| map.get("_raw.t-scoring"))
        .or_else(|| map.get("t-scoring.result"));
    match raw {
        Some(v) => unwrap_tool_node_content(v),
        None => serde_json::Value::Null,
    }
}

/// 从 snapshot / workflow results map 中提取 `t-valuation` 估值 JSON（**已解包**）。
///
/// 与 [`extract_score_json`] 同源：兼容三种 key + 6 层穿透。返回对象形如
/// `{current_price, dcf:{low,mid,high,assumptions,...}, graham:{...}, pe, pb, ...}`，
/// 供 DashboardReport 填充**估值语义**字段（`intrinsic_value_*` / `current_price`），
/// 使其与**交易语义**的 `targetPrice`（LLM trader 自填）在 UI 上分栏并列。
///
/// 2026-09-13（603466 风语筑）：仪表盘「目标价」显示 13.27（= 现价，LLM 自填），
/// 而估值区间是 4.44–5.57，用户读到「同一工作流结论矛盾」——
/// 根因是两条链共用一个词，**不是计算错误**。返回 `None` 时 UI 按「无估值数据」渲染。
pub(crate) fn extract_valuation_json(
    map: &std::collections::HashMap<String, serde_json::Value>,
) -> Option<serde_json::Value> {
    let raw = map
        .get("t-valuation")
        .or_else(|| map.get("_raw.t-valuation"))
        .or_else(|| map.get("t-valuation.result"))?;
    let unwrapped = unwrap_tool_node_content(raw);
    if unwrapped.is_object() {
        Some(unwrapped)
    } else {
        None
    }
}

/// 用 trader 的 LLM 决策补齐 dashboard 的绝对价格字段（**公式侧未产出时的回退**）。
///
/// ⚠️ **2026-09-23 起本函数的角色已变** —— 原文「这不是兜底，而是常态主路径」的判断**已作废**：
/// 2026-09-23 之前 `portfolio-mgr.rhai` 的 `decision_json` **只输出百分比**
/// （`stopLossPct`/`takeProfitPct`），**从不产出绝对价格** ⇒ 公式侧 `targetPrice`/`stopLoss`
/// 键永远不存在 ⇒ 本函数**恒命中**，仪表盘价位 **100% 来自 LLM 自填值**，公式侧无价可校验。
/// 于是 LLM 在「持有」档把 `targetPrice` 抄成 `currentPrice` 时（603466：13.27 == 13.27），
/// 仪表盘显示「等于现价的目标价」，与同一工作流的估值区间（DCF 4.44–5.57）读起来自相矛盾。
///
/// 该断链已在**源头**修复（2026-09-23）：`portfolio-mgr.rhai` 现按
/// `现价 × (1 ∓ 档位%)` 输出 `targetPrice`/`stopLoss` 绝对价格（档位取自 `timeHorizon`）。
/// 因此本函数的两条路径语义已分叉：
///
/// - **成功路径**：公式两键**存在且非 null** ⇒ 本函数的 `is_none_or(is_null)` 判据不成立
///   ⇒ **不覆盖** ⇒ **公式优先**。这是刻意的：公式价确定性、可复算、与 `timeHorizon` 自洽，
///   LLM 值不可校验。LLM 值只在公式**确实无交易计划**（`sl_pct <= 0`）时才机会入场。
/// - **异常路径**（rhai `catch` 块）：两键**存在但为 null** ⇒ 本函数仍会填入 LLM 值。
///   这是**已知残留**：一条「Rhai 执行异常」的记录不应携带任何交易结论。若要封死，
///   须在调用点先判 `action == "数据缺失"` 再决定是否合并（本轮未改）。
///
/// 配套修复：① trader prompt 补 `targetPrice` 方向语义并禁等于现价（模板 v39）；
/// ② `portfolio-mgr.rhai` 新增 R-204 判「价格信号无信息量」并留痕。
/// 本函数本身只影响 dashboard 显示，不改决策本体。
pub(crate) fn merge_price_fields_from_llm(
    decision_value: &serde_json::Value,
    llm_json: Option<&str>,
) -> serde_json::Value {
    let Some(s) = llm_json else { return decision_value.clone() };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(s) else {
        return decision_value.clone();
    };
    let mut dv = decision_value.clone();
    let Some(obj) = dv.as_object_mut() else { return dv };
    // llm 决策字段可能在顶层，也可能在 verdict 对象内（trader content = {report, verdict:{...}}）
    let find_field = |root: &serde_json::Value, key: &str| -> Option<serde_json::Value> {
        root.get(key).filter(|v| !v.is_null()).cloned().or_else(|| {
            root.get("verdict").and_then(|v| v.get(key)).filter(|v| !v.is_null()).cloned()
        })
    };
    if obj.get("targetPrice").is_none_or(|v| v.is_null()) {
        if let Some(v) = find_field(&parsed, "targetPrice") {
            obj.insert("targetPrice".to_string(), v);
        }
    }
    if obj.get("stopLoss").is_none_or(|v| v.is_null()) {
        if let Some(v) = find_field(&parsed, "stopLoss") {
            obj.insert("stopLoss".to_string(), v);
        }
    }
    dv
}

/// 从已有分析记录（DB 行）重建 DashboardReport + Markdown 文本。
///
/// DashboardReport 不持久化（stock_analyses 无 dashboard 列），只在 workflow-completed
/// 事件和 rerun_decision 返回时现场生成，导致重开历史分析时仪表盘 Tab 永远空态。
/// 本函数补上"加载历史时重建"的路径：DashboardReport 的三个输入
/// （decision + t-scoring 评分 + 分析师报告）全部在 `blackboard_snapshot` 里，
/// 无需重跑 Rhai 公式即可零成本恢复。
///
/// 返回 None 的情形：decision_json 为空/解析失败，或 blackboard_snapshot 解析失败
/// （旧记录 snapshot 损坏）—— 此时前端保持空态兜底。
pub(crate) fn build_dashboard_from_analysis_record(
    analysis: &stock_analyses::Model,
) -> Option<(axagent_harness::DashboardReport, String)> {
    // 1. 决策 JSON 必须存在且可解析（portfolio-mgr 输出）
    let decision_str = analysis.decision_json.as_deref()?;
    let decision_value: serde_json::Value =
        match serde_json::from_str::<serde_json::Value>(decision_str) {
            Ok(v) if !v.is_null() => v,
            _ => return None,
        };

    // 2. 解析 blackboard_snapshot → variables map，并将 _raw.{nodeId} 提升到顶层
    //    （与 rerun_decision 的解析逻辑保持一致）
    let snapshot_str = analysis.blackboard_snapshot.as_deref().unwrap_or("");
    let mut snapshot: std::collections::HashMap<String, serde_json::Value> =
        serde_json::from_str(snapshot_str).ok()?;
    let raw_keys: Vec<String> =
        snapshot.keys().filter(|k| k.starts_with("_raw.")).cloned().collect();
    for raw_key in raw_keys {
        if let Some(key) = raw_key.strip_prefix("_raw.") {
            if let Some(val) = snapshot.remove(&raw_key) {
                // 不覆盖已有 key（remapped key 优先，与 rerun_decision 一致）
                snapshot.entry(key.to_string()).or_insert(val);
            }
        }
    }

    // 3. 提取评分 JSON（穿透 ToolNode content 包装，取真实 {total, signal, ...}）
    let score_json = extract_score_json(&snapshot);

    // 4. 提取分析师报告
    let analyst_reports = extract_analyst_reports_from_snapshot(&snapshot);

    // 5. 构建 DashboardReport（目标价/止损价缺键时从 trader LLM 决策兜底）
    let dashboard_value =
        merge_price_fields_from_llm(&decision_value, analysis.llm_decision_json.as_deref());
    let dashboard_report =
        axagent_analysis_engine::dashboard_report::build_dashboard_report_from_workflow(
            &dashboard_value,
            &score_json,
            &analysis.stock_code,
            &analysis.stock_name,
            &analysis.analysis_date,
            &analyst_reports,
            extract_valuation_json(&snapshot).as_ref(),
        );
    let dashboard_md =
        axagent_analysis_engine::dashboard_report::render_dashboard_md(&dashboard_report);

    tracing::info!(
        "[build_dashboard_from_analysis_record] DashboardReport 重建完成: \
         analysis_id={}, integrity_passed={}, risk_alerts={}, catalysts={}",
        analysis.id,
        dashboard_report.integrity_passed,
        dashboard_report.risk_alerts.len(),
        dashboard_report.catalysts.len()
    );

    Some((dashboard_report, dashboard_md))
}
