use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

use axagent_agent::shared_blackboard::SharedBlackboard;
use axagent_astock_data::{AStockClient, StockRawData};

use crate::decision::{AgentRunner, AnalysisConfig, AnalysisEvent, StockDecision};
use crate::pipeline;
use crate::prompts;

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
const RISK_IDS: &[&str] = &[
    "aggressive-debator",
    "conservative-debator",
    "neutral-debator",
];
const RISK_MANAGER_ID: &str = "research-manager";

/// 决策角色
const PORTFOLIO_MANAGER_ID: &str = "portfolio-manager";

// ── 系统提示（回退）──
// 当 Markdown 文件未找到时使用的简化回退提示词

fn fallback_prompt(expert_id: &str) -> String {
    format!("你是{}。基于提供的数据进行分析。只输出JSON格式结果。", expert_id)
}

/// 股票分析编排器 — 5 阶段执行
pub struct StockAnalysisOrchestrator;

impl StockAnalysisOrchestrator {
    /// 运行完整的 5 阶段分析
    ///
    /// * `prompts` - 从 Markdown 文件加载的专家提示词映射
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
        prompts: HashMap<String, String>,
        cancel_token: Option<Arc<AtomicBool>>,
    ) -> Result<StockDecision, String> {
        // 将提示词包装为 Arc 以便在 spawn 任务中共享
        let prompts = Arc::new(prompts);
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
            .inspect_err(|e| {
                let _ = events.send(AnalysisEvent::Error {
                    stage: "data_loading".into(),
                    message: e.clone(),
                });
            })?;

        // 取消检查
        if Self::is_cancelled(&cancel_token) {
            return Err("分析已取消".into());
        }

        // ── 阶段 2: 7 位分析师并行 ──
        Self::phase_2_analysts(&runner, &blackboard, &events, &prompts, &cancel_token).await?;

        // ── 阶段 3: 多空辩论 ──
        Self::phase_3_debate(
            &runner,
            &blackboard,
            &events,
            config.max_debate_rounds,
            &prompts,
            &cancel_token,
        )
        .await?;

        // ── 阶段 4: 风险评估 ──
        Self::phase_4_risk(&runner, &blackboard, &events, &prompts, &cancel_token).await?;

        // ── 阶段 5: 投资决策 ──
        let decision =
            Self::phase_5_decision(&runner, &blackboard, &events, &prompts, &cancel_token).await?;

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
        prompts: &Arc<HashMap<String, String>>,
        cancel_token: &Option<Arc<AtomicBool>>,
    ) -> Result<(), String> {
        let mut handles = Vec::new();

        for &analyst_id in ANALYST_IDS {
            let id = analyst_id.to_string();
            let bb = blackboard.clone();
            let ev = events.clone();
            let r = runner.clone();
            let p = prompts.clone();
            let ct = cancel_token.clone();

            handles.push(tokio::spawn(async move {
                Self::run_single_analyst(&r, &id, &bb, &ev, &p, &ct).await
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
        prompts: &Arc<HashMap<String, String>>,
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
        let sys_prompt = prompts::get_analyst_context(expert_id, prompts)
            .unwrap_or_else(|| fallback_prompt(expert_id));

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
    //
    // 设计说明：为何未使用 axagent_runtime::adversarial_debate::DebateManager？
    //
    // 共享的 DebateManager 是一个无 LLM 能力的纯数据结构——它接收外部传入的
    // argument/strength 值并跟踪回合、计算得分、检测收敛。其 evaluate_strength()
    // 基于关键词（"data"/"because"）的启发式规则，不适用于需要深度推理的金融分析场景。
    //
    // 本编排器的辩论阶段需要：
    // 1. 每轮调用 LLM 生成 bull/bear 论证（DebateManager 不支持）
    // 2. 将分析师报告和对手上一轮论证作为丰富上下文注入 LLM prompt
    // 3. 通过 SharedBlackboard 存储和共享辩论状态
    // 4. 通过 AnalysisEvent broadcast 向 UI 推送辩论进度
    //
    // 因此，保留自定义辩论循环是更合理的选择。DebateManager 定位为通用多议题
    // 辩论跟踪器，适用于不需要 LLM 推理的简单辩论计分场景。

    async fn phase_3_debate(
        runner: &Option<Arc<dyn AgentRunner>>,
        blackboard: &Arc<RwLock<SharedBlackboard>>,
        events: &tokio::sync::broadcast::Sender<AnalysisEvent>,
        max_rounds: u32,
        prompts: &Arc<HashMap<String, String>>,
        cancel_token: &Option<Arc<AtomicBool>>,
    ) -> Result<(), String> {
        let sys_bull = prompts::get_analyst_context(BULL_ID, prompts)
            .unwrap_or_else(|| fallback_prompt(BULL_ID));
        let sys_bear = prompts::get_analyst_context(BEAR_ID, prompts)
            .unwrap_or_else(|| fallback_prompt(BEAR_ID));

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
        prompts: &Arc<HashMap<String, String>>,
        cancel_token: &Option<Arc<AtomicBool>>,
    ) -> Result<(), String> {
        // 3 个风险评估员并行执行
        let mut handles = Vec::new();
        for &risk_id in RISK_IDS {
            let id = risk_id.to_string();
            let bb = blackboard.clone();
            let ev = events.clone();
            let r = runner.clone();
            let p = prompts.clone();
            let sys = prompts::get_analyst_context(risk_id, &p)
                .unwrap_or_else(|| fallback_prompt(risk_id));
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

        let sys_manager = prompts::get_analyst_context(RISK_MANAGER_ID, prompts)
            .unwrap_or_else(|| fallback_prompt(RISK_MANAGER_ID));

        let manager_report = if let Some(ref r) = runner {
            r.run_agent(RISK_MANAGER_ID, &sys_manager, &manager_ctx)
                .await?
        } else {
            r#"{"expert":"research-manager","summary":"综合风险评估占位报告","risk_balance":"中性偏谨慎","key_risks":["占位风险"],"key_opportunities":["占位机会"]}"#.to_string()
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
        prompts: &Arc<HashMap<String, String>>,
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
        let sys_prompt = prompts::get_analyst_context(PORTFOLIO_MANAGER_ID, prompts)
            .unwrap_or_else(|| fallback_prompt(PORTFOLIO_MANAGER_ID));

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
