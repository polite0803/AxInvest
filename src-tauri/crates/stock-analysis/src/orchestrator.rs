use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

use axagent_agent::shared_blackboard::SharedBlackboard;
use axagent_astock_data::{AStockClient, StockRawData};

use crate::decision::{AgentRunner, AnalysisConfig, AnalysisEvent, StockDecision};
use crate::pipeline;

/// 7 个分析师专家 ID
pub const ANALYST_IDS: &[&str] = &[
    "market-analyst",
    "sentiment-analyst",
    "news-analyst",
    "fundamentals-analyst",
    "policy-analyst",
    "hot-money-tracker",
    "lockup-watcher",
];

/// 辩论角色
const BULL_ID: &str = "bull-researcher";
const BEAR_ID: &str = "bear-researcher";

/// 风控评估员
const RISK_IDS: &[&str] = &["liquidity-risk", "market-risk", "credit-risk"];
const RISK_MANAGER_ID: &str = "research-manager";

/// 决策角色
const PORTFOLIO_MANAGER_ID: &str = "portfolio-manager";

// ── 系统提示 ──

fn system_prompt(expert_id: &str) -> &'static str {
    match expert_id {
        "market-analyst" => "你是A股市场技术分析师。基于提供的K线数据，进行技术面分析：趋势判断（均线系统、MACD、布林带）、量价关系、支撑压力位、形态识别。输出结构化分析报告。只输出JSON格式结果。",
        "sentiment-analyst" => "你是A股市场情绪分析师。基于新闻、资金流向、龙虎榜数据，分析市场情绪：散户情绪、机构动向、舆情热度、恐慌/贪婪指数。输出结构化分析报告。只输出JSON格式结果。",
        "news-analyst" => "你是A股新闻分析师。基于新闻数据，分析近期重大新闻事件及其对股价的潜在影响：利好/利空分类、事件驱动逻辑、时效性评估。输出结构化分析报告。只输出JSON格式结果。",
        "fundamentals-analyst" => "你是A股基本面分析师。基于财务数据，进行基本面评估：盈利能力（ROE、毛利率、净利率）、成长性（营收/利润增速）、估值水平（PE/PB分位）、财务健康度。输出结构化分析报告。只输出JSON格式结果。",
        "policy-analyst" => "你是A股政策分析师。基于新闻和行业数据，分析宏观政策和行业政策对股票的影响：货币政策、财政政策、行业监管、产业扶持、地缘政治风险。输出结构化分析报告。只输出JSON格式结果。",
        "hot-money-tracker" => "你是A股资金面分析师。基于资金流向和龙虎榜数据，分析主力资金动向：北向资金、游资席位、机构席位、大单流向、融资融券。输出结构化分析报告。只输出JSON格式结果。",
        "lockup-watcher" => "你是A股限售解禁分析师。基于限售解禁数据，分析解禁对股价的压力：解禁规模、解禁比例、解禁股东类型（大股东/机构/战略投资者）、历史解禁影响。输出结构化分析报告。只输出JSON格式结果。",
        BULL_ID => "你是A股多方研究员。你的任务是从乐观角度论证该股票的投资价值。基于各分析师报告中的数据，构建看多逻辑：成长驱动、估值修复、催化因素、市场情绪。输出结构化辩论报告。只输出JSON格式结果。",
        BEAR_ID => "你是A股空方研究员。你的任务是从风险角度论证该股票的下行风险。基于各分析师报告中的数据，构建看空逻辑：估值泡沫、业绩风险、竞争压力、政策风险。输出结构化辩论报告。只输出JSON格式结果。",
        "liquidity-risk" => "你是流动性风险评估师。基于交易数据和技术面报告，评估流动性风险：日均换手率、买卖盘深度、大单占比、量价异常。输出结构化评估报告。只输出JSON格式结果。",
        "market-risk" => "你是市场风险评估师。基于宏观数据和技术面报告，评估系统性市场风险：大盘走势、行业轮动、外围市场影响、波动率。输出结构化评估报告。只输出JSON格式结果。",
        "credit-risk" => "你是信用风险评估师。基于财务报告和基本面分析，评估信用风险：偿债能力、现金流健康度、质押比例、商誉减值风险。输出结构化评估报告。只输出JSON格式结果。",
        RISK_MANAGER_ID => "你是研究主管。你的任务是将多方/空方辩论论点及各风险评估报告进行综合，提炼出关键风险和机会的平衡视图。为投资组合经理提供最终摘要。输出结构化综合报告。只输出JSON格式结果。",
        PORTFOLIO_MANAGER_ID => "你是A股投资组合经理。基于所有分析报告（7位分析师、多空辩论、风险评估），做出最终投资决策。必须输出严格JSON：{\"action\":\"买入/增持/持有/减持/卖出\",\"position_pct\":0-100,\"target_price\":数字,\"stop_loss\":数字,\"reasoning\":\"理由\",\"risk_level\":\"低/中/高\",\"confidence\":0-1}。只输出JSON格式结果。",
        _ => "你是股票分析专家。基于提供的数据进行分析。只输出JSON格式结果。",
    }
}

/// 股票分析编排器 — 5 阶段执行
pub struct StockAnalysisOrchestrator;

impl StockAnalysisOrchestrator {
    /// 运行完整的 5 阶段分析
    ///
    /// * `runner` - LLM Agent 执行器，`None` 时生成占位报告
    /// * `cancel_token` - 取消令牌，检查于各阶段之间
    #[allow(clippy::too_many_arguments)]
    pub async fn run(
        data_client: &AStockClient,
        blackboard: Arc<RwLock<SharedBlackboard>>,
        stock_code: String,
        stock_name: String,
        date: String,
        config: AnalysisConfig,
        events: tokio::sync::broadcast::Sender<AnalysisEvent>,
        runner: Option<Arc<dyn AgentRunner>>,
        cancel_token: Option<Arc<AtomicBool>>,
    ) -> Result<StockDecision, String> {
        // 验证配置
        config.validate().map_err(|e| format!("配置无效: {}", e))?;

        // 写入基本元数据
        {
            let mut bb = blackboard.write().await;
            bb.set_state("stock_code", &stock_code);
            bb.set_state("stock_name", &stock_name);
            bb.set_state("analysis_date", &date);
        }

        let _ = events.send(AnalysisEvent::Started {
            stock_code: stock_code.clone(),
            stock_name: stock_name.clone(),
            date: date.clone(),
        });

        // ── 阶段 1: 数据加载 ──
        Self::phase_1_load_data(data_client, &stock_code, &config, &blackboard, &events)
            .await
            .map_err(|e| {
                let _ = events.send(AnalysisEvent::Error {
                    stage: "data_loading".into(),
                    message: e.clone(),
                });
                e
            })?;

        // 取消检查
        if Self::is_cancelled(&cancel_token) {
            return Err("分析已取消".into());
        }

        // ── 阶段 2: 7 位分析师并行 ──
        Self::phase_2_analysts(&runner, &blackboard, &events, &cancel_token).await?;

        // ── 阶段 3: 多空辩论 ──
        Self::phase_3_debate(
            &runner,
            &blackboard,
            &events,
            config.max_debate_rounds,
            &cancel_token,
        )
        .await?;

        // ── 阶段 4: 风险评估 ──
        Self::phase_4_risk(&runner, &blackboard, &events, &cancel_token).await?;

        // ── 阶段 5: 投资决策 ──
        let decision = Self::phase_5_decision(&runner, &blackboard, &events, &cancel_token).await?;

        let _ = events.send(AnalysisEvent::Decision(decision.clone()));

        Ok(decision)
    }

    // ── 取消检查 ──

    fn is_cancelled(token: &Option<Arc<AtomicBool>>) -> bool {
        token
            .as_ref()
            .map(|t| t.load(Ordering::Relaxed))
            .unwrap_or(false)
    }

    // ── 阶段 1 ──

    async fn phase_1_load_data(
        data_client: &AStockClient,
        stock_code: &str,
        config: &AnalysisConfig,
        blackboard: &Arc<RwLock<SharedBlackboard>>,
        events: &tokio::sync::broadcast::Sender<AnalysisEvent>,
    ) -> Result<StockRawData, String> {
        let raw = data_client
            .fetch_all(stock_code, &config.kline_period, config.kline_limit, config.news_limit)
            .await
            .map_err(|e| format!("数据获取失败: {}", e))?;

        let klines_json = serde_json::to_string(&raw.klines).unwrap_or_default();
        let financials_json = serde_json::to_string(&raw.financials).unwrap_or_default();
        let news_json = serde_json::to_string(&raw.news).unwrap_or_default();
        let money_flow_json = raw
            .money_flow
            .as_ref()
            .map(|m| serde_json::to_string(m).unwrap_or_default())
            .unwrap_or_default();
        let dragon_tiger_json = serde_json::to_string(&raw.dragon_tiger).unwrap_or_default();
        let lockup_json = serde_json::to_string(&raw.lockup).unwrap_or_default();

        {
            let mut bb = blackboard.write().await;
            bb.set_state("raw.klines", &klines_json);
            bb.set_state("raw.financials", &financials_json);
            bb.set_state("raw.news", &news_json);
            bb.set_state("raw.money_flow", &money_flow_json);
            bb.set_state("raw.dragon_tiger", &dragon_tiger_json);
            bb.set_state("raw.lockup", &lockup_json);
        }

        let _ = events.send(AnalysisEvent::DataLoaded {
            kline_count: raw.klines.len(),
            news_count: raw.news.len(),
        });

        Ok(raw)
    }

    // ── 阶段 2: 7 位分析师并行执行 ──

    async fn phase_2_analysts(
        runner: &Option<Arc<dyn AgentRunner>>,
        blackboard: &Arc<RwLock<SharedBlackboard>>,
        events: &tokio::sync::broadcast::Sender<AnalysisEvent>,
        cancel_token: &Option<Arc<AtomicBool>>,
    ) -> Result<(), String> {
        let mut handles = Vec::new();

        for &analyst_id in ANALYST_IDS {
            let id = analyst_id.to_string();
            let bb = blackboard.clone();
            let ev = events.clone();
            let r = runner.clone();
            let ct = cancel_token.clone();

            handles.push(tokio::spawn(async move {
                Self::run_single_analyst(&r, &id, &bb, &ev, &ct).await
            }));
        }

        for handle in handles {
            match handle.await {
                Ok(Ok(report)) => {
                    tracing::info!("分析师报告生成成功");
                    drop(report); // 已在 run_single_analyst 中发送事件
                },
                Ok(Err(e)) => {
                    tracing::warn!("分析师执行失败: {}", e);
                },
                Err(e) => {
                    tracing::warn!("分析师 task panic: {}", e);
                },
            }
        }

        Ok(())
    }

    async fn run_single_analyst(
        runner: &Option<Arc<dyn AgentRunner>>,
        expert_id: &str,
        blackboard: &Arc<RwLock<SharedBlackboard>>,
        events: &tokio::sync::broadcast::Sender<AnalysisEvent>,
        cancel_token: &Option<Arc<AtomicBool>>,
    ) -> Result<String, String> {
        // 进度通知
        let _ = events.send(AnalysisEvent::AnalystProgress {
            expert_id: expert_id.to_string(),
            status: "正在分析...".into(),
            progress_pct: 0,
        });

        if Self::is_cancelled(cancel_token) {
            return Err("已取消".into());
        }

        // 构建数据上下文
        let user_prompt = pipeline::build_analyst_context(expert_id, blackboard).await;
        let sys_prompt = system_prompt(expert_id).to_string();

        let report = if let Some(ref r) = runner {
            // 通过 AgentRunner 执行 LLM 分析
            let _ = events.send(AnalysisEvent::AnalystProgress {
                expert_id: expert_id.to_string(),
                status: "调用LLM...".into(),
                progress_pct: 50,
            });
            r.run_agent(expert_id, &sys_prompt, &user_prompt).await?
        } else {
            // 无 runner 时生成结构化占位报告
            Self::placeholder_analyst_report(expert_id)
        };

        // 写入 Blackboard 并发送事件
        pipeline::write_report(expert_id, &report, blackboard, events).await;

        Ok(report)
    }

    /// 当 AgentRunner 未注入时，生成包含数据摘要的占位报告
    fn placeholder_analyst_report(expert_id: &str) -> String {
        let label = match expert_id {
            "market-analyst" => "技术面分析",
            "sentiment-analyst" => "情绪分析",
            "news-analyst" => "新闻分析",
            "fundamentals-analyst" => "基本面分析",
            "policy-analyst" => "政策分析",
            "hot-money-tracker" => "资金面分析",
            "lockup-watcher" => "限售解禁分析",
            _ => "分析",
        };
        format!(
            r#"{{"expert":"{}","type":"{}","summary":"占位报告 — AgentRunner 未注入。数据已加载至 Blackboard，待 LLM 集成后生成真实分析。","signals":[],"risk_flags":[]}}"#,
            expert_id, label
        )
    }

    // ── 阶段 3: 多空辩论 ──

    async fn phase_3_debate(
        runner: &Option<Arc<dyn AgentRunner>>,
        blackboard: &Arc<RwLock<SharedBlackboard>>,
        events: &tokio::sync::broadcast::Sender<AnalysisEvent>,
        max_rounds: u32,
        cancel_token: &Option<Arc<AtomicBool>>,
    ) -> Result<(), String> {
        let sys_bull = system_prompt(BULL_ID).to_string();
        let sys_bear = system_prompt(BEAR_ID).to_string();

        let mut bull_prev = String::new();
        let mut bear_prev = String::new();

        for round in 1..=max_rounds {
            if Self::is_cancelled(cancel_token) {
                return Err("已取消".into());
            }

            let _ = events.send(AnalysisEvent::AnalystProgress {
                expert_id: "debate".into(),
                status: format!("辩论第 {}/{} 轮", round, max_rounds),
                progress_pct: ((round as u8 * 100) / max_rounds as u8),
            });

            // 多方论证：读取空方上一轮论点作为反驳依据
            let bull_context = Self::build_debate_context(
                BULL_ID,
                blackboard,
                if round == 1 { None } else { Some(&bear_prev) },
            )
            .await;

            let bull_arg = if let Some(ref r) = runner {
                r.run_agent(BULL_ID, &sys_bull, &bull_context).await?
            } else {
                format!(
                    r#"{{"round":{},"role":"bull","argument":"多方辩论占位 — 第{}轮","key_points":["技术面支持","基本面良好"],"confidence":0.6}}"#,
                    round, round
                )
            };
            bull_prev = bull_arg.clone();
            {
                let mut bb = blackboard.write().await;
                bb.set_state(&format!("debate.bull.round_{}", round), &bull_arg);
            }

            if Self::is_cancelled(cancel_token) {
                return Err("已取消".into());
            }

            // 空方论证：读取多方本轮论点
            let bear_context =
                Self::build_debate_context(BEAR_ID, blackboard, Some(&bull_arg)).await;

            let bear_arg = if let Some(ref r) = runner {
                r.run_agent(BEAR_ID, &sys_bear, &bear_context).await?
            } else {
                format!(
                    r#"{{"round":{},"role":"bear","argument":"空方辩论占位 — 第{}轮","key_points":["估值偏高","政策不确定性"],"confidence":0.5}}"#,
                    round, round
                )
            };
            bear_prev = bear_arg.clone();
            {
                let mut bb = blackboard.write().await;
                bb.set_state(&format!("debate.bear.round_{}", round), &bear_arg);
            }

            let _ = events.send(AnalysisEvent::DebateRound {
                round,
                bull_argument: bull_arg,
                bear_argument: bear_arg,
            });
        }

        // 写入最终辩论摘要
        let summary = format!(
            r#"{{"rounds":{},"bull_final":"{}","bear_final":"{}"}}"#,
            max_rounds,
            &bull_prev[..bull_prev.len().min(200)],
            &bear_prev[..bear_prev.len().min(200)]
        );
        {
            let mut bb = blackboard.write().await;
            bb.set_state("debate.summary", &summary);
            bb.set_state("report.bull-researcher", &bull_prev);
            bb.set_state("report.bear-researcher", &bear_prev);
        }

        Ok(())
    }

    async fn build_debate_context(
        role: &str,
        blackboard: &Arc<RwLock<SharedBlackboard>>,
        opponent_arg: Option<&str>,
    ) -> String {
        let mut ctx = String::new();
        ctx.push_str(&format!("角色: {}\n\n", role));

        // 读取所有分析师报告
        let bb = blackboard.read().await;
        for analyst_id in ANALYST_IDS {
            let field = format!("report.{}", analyst_id);
            if let Some(report) = bb.get_state(&field) {
                ctx.push_str(&format!(
                    "--- {} 报告 ---\n{}\n",
                    analyst_id,
                    if report.len() > 500 {
                        &report[..500]
                    } else {
                        report
                    }
                ));
            }
        }

        // 读取对方上一轮论点
        if let Some(arg) = opponent_arg {
            ctx.push_str(&format!(
                "\n--- 对手上一轮论点 ---\n{}\n",
                if arg.len() > 500 { &arg[..500] } else { arg }
            ));
        }

        ctx
    }

    // ── 阶段 4: 风险评估 ──

    async fn phase_4_risk(
        runner: &Option<Arc<dyn AgentRunner>>,
        blackboard: &Arc<RwLock<SharedBlackboard>>,
        events: &tokio::sync::broadcast::Sender<AnalysisEvent>,
        cancel_token: &Option<Arc<AtomicBool>>,
    ) -> Result<(), String> {
        // 3 个风险评估员并行执行
        let mut handles = Vec::new();
        for &risk_id in RISK_IDS {
            let id = risk_id.to_string();
            let bb = blackboard.clone();
            let ev = events.clone();
            let r = runner.clone();
            let sys = system_prompt(risk_id).to_string();
            let ct = cancel_token.clone();

            handles.push(tokio::spawn(async move {
                if Self::is_cancelled(&ct) {
                    return Err("已取消".into());
                }

                let user_prompt = pipeline::build_analyst_context(&id, &bb).await;

                let report = if let Some(ref runner) = r {
                    runner.run_agent(&id, &sys, &user_prompt).await?
                } else {
                    Self::placeholder_analyst_report(&id)
                };

                pipeline::write_report(&id, &report, &bb, &ev).await;

                let _ = ev.send(AnalysisEvent::RiskAssessment {
                    risk_type: id.clone(),
                    report: report.clone(),
                });

                Ok::<String, String>(report)
            }));
        }

        let mut risk_reports = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(Ok(report)) => risk_reports.push(report),
                Ok(Err(e)) => tracing::warn!("风险评估员执行失败: {}", e),
                Err(e) => tracing::warn!("风险评估员 task panic: {}", e),
            }
        }

        if Self::is_cancelled(cancel_token) {
            return Err("已取消".into());
        }

        // 研究主管综合所有风险报告
        let manager_ctx = {
            let bb = blackboard.read().await;
            let mut ctx = String::new();
            ctx.push_str(&format!("角色: {}\n\n", RISK_MANAGER_ID));
            for (i, report) in risk_reports.iter().enumerate() {
                ctx.push_str(&format!(
                    "--- {} 评估报告 ---\n{}\n\n",
                    RISK_IDS.get(i).unwrap_or(&"unknown"),
                    report
                ));
            }
            // 也包含辩论摘要
            if let Some(debate) = bb.get_state("debate.summary") {
                ctx.push_str(&format!("\n--- 多空辩论摘要 ---\n{}\n", debate));
            }
            ctx
        };

        let sys_manager = system_prompt(RISK_MANAGER_ID).to_string();

        let manager_report = if let Some(ref r) = runner {
            r.run_agent(RISK_MANAGER_ID, &sys_manager, &manager_ctx)
                .await?
        } else {
            format!(
                r#"{{"expert":"research-manager","summary":"综合风险评估占位报告","risk_balance":"中性偏谨慎","key_risks":["占位风险"],"key_opportunities":["占位机会"]}}"#
            )
        };

        pipeline::write_report(RISK_MANAGER_ID, &manager_report, blackboard, events).await;

        let _ = events.send(AnalysisEvent::RiskAssessment {
            risk_type: "comprehensive".into(),
            report: manager_report,
        });

        Ok(())
    }

    // ── 阶段 5: 投资决策 ──

    async fn phase_5_decision(
        runner: &Option<Arc<dyn AgentRunner>>,
        blackboard: &Arc<RwLock<SharedBlackboard>>,
        events: &tokio::sync::broadcast::Sender<AnalysisEvent>,
        cancel_token: &Option<Arc<AtomicBool>>,
    ) -> Result<StockDecision, String> {
        if Self::is_cancelled(cancel_token) {
            return Err("已取消".into());
        }

        let _ = events.send(AnalysisEvent::AnalystProgress {
            expert_id: PORTFOLIO_MANAGER_ID.into(),
            status: "综合分析生成决策...".into(),
            progress_pct: 90,
        });

        // 构建决策上下文（所有报告）
        let user_prompt = pipeline::build_analyst_context(PORTFOLIO_MANAGER_ID, blackboard).await;
        let sys_prompt = system_prompt(PORTFOLIO_MANAGER_ID).to_string();

        let decision_text = if let Some(ref r) = runner {
            r.run_agent(PORTFOLIO_MANAGER_ID, &sys_prompt, &user_prompt)
                .await?
        } else {
            // 占位决策
            r#"{"action":"持有","position_pct":0,"target_price":null,"stop_loss":null,"reasoning":"分析框架已完成，待 AgentRunner 注入后生成真实 LLM 决策。","risk_level":"中","confidence":0.0}"#.to_string()
        };

        // 尝试解析 LLM 输出的 JSON 决策
        let decision = Self::parse_decision(&decision_text).unwrap_or_else(|e| {
            tracing::warn!("决策解析失败，使用默认值: {}", e);
            StockDecision {
                action: "持有".into(),
                position_pct: 0.0,
                target_price: None,
                stop_loss: None,
                reasoning: format!(
                    "解析失败: {}。原始输出: {}",
                    e,
                    &decision_text[..200.min(decision_text.len())]
                ),
                risk_level: "中".into(),
                confidence: 0.0,
            }
        });

        // 写入 Blackboard
        {
            let mut bb = blackboard.write().await;
            bb.set_state("report.portfolio-manager", &decision_text);
            if let Ok(json) = serde_json::to_string(&decision) {
                bb.set_state("decision.final", &json);
            }
        }

        let _ = events.send(AnalysisEvent::InvestmentPlan {
            plan: decision.reasoning.clone(),
        });

        Ok(decision)
    }

    /// 从 LLM 输出文本中提取 JSON 并解析为 StockDecision
    fn parse_decision(text: &str) -> Result<StockDecision, String> {
        // 尝试直接解析
        if let Ok(d) = serde_json::from_str::<StockDecision>(text) {
            return Ok(d);
        }

        // 尝试提取 ```json ... ``` 代码块
        if let Some(start) = text.find("```json") {
            let inner = &text[start + 7..];
            if let Some(end) = inner.find("```") {
                let json_str = &inner[..end].trim();
                if let Ok(d) = serde_json::from_str::<StockDecision>(json_str) {
                    return Ok(d);
                }
            }
        }

        // 尝试提取 { ... } JSON 对象
        if let Some(start) = text.find('{') {
            if let Some(end) = text.rfind('}') {
                let json_str = &text[start..=end];
                if let Ok(d) = serde_json::from_str::<StockDecision>(json_str) {
                    return Ok(d);
                }
            }
        }

        Err("无法从输出中提取有效 JSON 决策".into())
    }
}
