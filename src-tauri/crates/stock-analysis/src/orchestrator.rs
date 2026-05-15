use std::collections::HashMap;
use std::fmt::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

use axagent_agent::shared_blackboard::SharedBlackboard;
use axagent_astock_data::{AStockClient, StockRawData};

use crate::decision::{AgentRunner, AnalysisConfig, AnalysisEvent, StockDecision};
use crate::pipeline;
use crate::prompts;

pub const ANALYST_IDS: &[&str] = &[
    "market-analyst",
    "sentiment-analyst",
    "news-analyst",
    "fundamentals-analyst",
    "policy-analyst",
    "hot-money-tracker",
    "lockup-watcher",
];

const BULL_ID: &str = "bull-researcher";
const BEAR_ID: &str = "bear-researcher";

const RISK_IDS: &[&str] = &[
    "aggressive-debator",
    "conservative-debator",
    "neutral-debator",
];
const RISK_MANAGER_ID: &str = "research-manager";

const TRADER_ID: &str = "trader";
const PORTFOLIO_MANAGER_ID: &str = "portfolio-manager";

fn fallback_prompt(expert_id: &str) -> String {
    format!("你是{expert_id}。基于提供的数据进行分析。只输出JSON格式结果。")
}

pub struct StockAnalysisOrchestrator;

impl StockAnalysisOrchestrator {
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
        let prompts = Arc::new(prompts);
        config.validate().map_err(|e| format!("配置无效: {e}"))?;

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

        Self::phase_1_load_data(data_client, &stock_code, &config, &blackboard, &events)
            .await
            .inspect_err(|e| {
                let _ = events.send(AnalysisEvent::Error {
                    stage: "data_loading".into(),
                    message: e.clone(),
                });
            })?;

        if Self::is_cancelled(&cancel_token) {
            return Err("分析已取消".into());
        }

        Self::phase_2_analysts(&runner, &blackboard, &events, &prompts, &cancel_token).await?;

        // 数据质量门控：在辩论/风控/决策前注入质量摘要
        {
            let reports = {
                let bb = blackboard.read().await;
                let mut map = HashMap::new();
                for id in ANALYST_IDS {
                    if let Some(report) = bb.get_state(&format!("report.{id}")) {
                        map.insert(id.to_string(), report.clone());
                    }
                }
                map
            };
            let quality = crate::quality::run_quality_gate(&reports);
            tracing::info!("{}", quality.summary);
            {
                let mut bb = blackboard.write().await;
                bb.set_state("data_quality_summary", &quality.summary);
            }
        }

        Self::phase_3_debate(
            &runner,
            &blackboard,
            &events,
            config.max_debate_rounds,
            &prompts,
            &cancel_token,
        )
        .await?;

        Self::phase_4_risk(&runner, &blackboard, &events, &prompts, &cancel_token).await?;

        Self::phase_4b_trader(&runner, &blackboard, &events, &prompts, &cancel_token).await?;

        let decision =
            Self::phase_5_decision(&runner, &blackboard, &events, &prompts, &cancel_token).await?;

        let _ = events.send(AnalysisEvent::Decision(decision.clone()));

        Ok(decision)
    }

    fn is_cancelled(token: &Option<Arc<AtomicBool>>) -> bool {
        token
            .as_ref()
            .map(|t| t.load(Ordering::Relaxed))
            .unwrap_or(false)
    }

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
            .map_err(|e| format!("数据获取失败: {e}"))?;

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
                    drop(report);
                },
                Ok(Err(e)) => {
                    tracing::warn!("分析师执行失败: {e}");
                },
                Err(e) => {
                    tracing::warn!("分析师 task panic: {e}");
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
        let _ = events.send(AnalysisEvent::AnalystProgress {
            expert_id: expert_id.to_string(),
            status: "正在分析...".into(),
            progress_pct: 0,
        });

        if Self::is_cancelled(cancel_token) {
            return Err("已取消".into());
        }

        let user_prompt = pipeline::build_analyst_context(expert_id, blackboard).await;
        let sys_prompt = prompts::get_analyst_context(expert_id, prompts)
            .unwrap_or_else(|| fallback_prompt(expert_id));

        let report = if let Some(ref r) = runner {
            let _ = events.send(AnalysisEvent::AnalystProgress {
                expert_id: expert_id.to_string(),
                status: "调用LLM...".into(),
                progress_pct: 50,
            });
            r.run_agent(expert_id, &sys_prompt, &user_prompt).await?
        } else {
            Self::placeholder_analyst_report(expert_id)
        };

        pipeline::write_report(expert_id, &report, blackboard, events).await;

        Ok(report)
    }

    fn placeholder_analyst_report(expert_id: &str) -> String {
        let label = match expert_id {
            "market-analyst" => "技术面分析",
            "sentiment-analyst" => "情绪分析",
            "news-analyst" => "新闻分析",
            "fundamentals-analyst" => "基本面分析",
            "policy-analyst" => "政策分析",
            "hot-money-tracker" => "资金面分析",
            "lockup-watcher" => "限售解禁分析",
            "trader" => "交易执行方案",
            _ => "分析",
        };
        format!(
            r#"{{"expert":"{expert_id}","type":"{label}","summary":"占位报告 — AgentRunner 未注入。数据已加载至 Blackboard，待 LLM 集成后生成真实分析。","signals":[],"risk_flags":[]}}"#
        )
    }

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
                status: format!("辩论第 {round}/{max_rounds} 轮"),
                progress_pct: (round * 100 / max_rounds).min(100) as u8,
            });

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
                    r#"{{"round":{round},"role":"bull","argument":"多方辩论占位 — 第{round}轮","key_points":["技术面支持","基本面良好"],"confidence":0.6}}"#
                )
            };
            bull_prev = bull_arg.clone();
            {
                let mut bb = blackboard.write().await;
                bb.set_state(&format!("debate.bull.round_{round}"), &bull_arg);
            }

            if Self::is_cancelled(cancel_token) {
                return Err("已取消".into());
            }

            let bear_context =
                Self::build_debate_context(BEAR_ID, blackboard, Some(&bull_arg)).await;

            let bear_arg = if let Some(ref r) = runner {
                r.run_agent(BEAR_ID, &sys_bear, &bear_context).await?
            } else {
                format!(
                    r#"{{"round":{round},"role":"bear","argument":"空方辩论占位 — 第{round}轮","key_points":["估值偏高","政策不确定性"],"confidence":0.5}}"#
                )
            };
            bear_prev = bear_arg.clone();
            {
                let mut bb = blackboard.write().await;
                bb.set_state(&format!("debate.bear.round_{round}"), &bear_arg);
            }

            let _ = events.send(AnalysisEvent::DebateRound {
                round,
                bull_argument: bull_arg,
                bear_argument: bear_arg,
            });
        }

        let summary = format!(
            r#"{{"rounds":{max_rounds},"bull_final":"{}","bear_final":"{}"}}"#,
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
        let _ = write!(ctx, "角色: {role}\n\n");

        let bb = blackboard.read().await;
        for analyst_id in ANALYST_IDS {
            let field = format!("report.{analyst_id}");
            if let Some(report) = bb.get_state(&field) {
                let _ = write!(
                    ctx,
                    "--- {} 报告 ---\n{}\n",
                    analyst_id,
                    if report.len() > 500 {
                        &report[..500]
                    } else {
                        report
                    }
                );
            }
        }

        if let Some(arg) = opponent_arg {
            let _ = write!(
                ctx,
                "\n--- 对手上一轮论点 ---\n{}\n",
                if arg.len() > 500 { &arg[..500] } else { arg }
            );
        }

        ctx
    }

    async fn phase_4_risk(
        runner: &Option<Arc<dyn AgentRunner>>,
        blackboard: &Arc<RwLock<SharedBlackboard>>,
        events: &tokio::sync::broadcast::Sender<AnalysisEvent>,
        prompts: &Arc<HashMap<String, String>>,
        cancel_token: &Option<Arc<AtomicBool>>,
    ) -> Result<(), String> {
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
                Ok(Err(e)) => tracing::warn!("风险评估员执行失败: {e}"),
                Err(e) => tracing::warn!("风险评估员 task panic: {e}"),
            }
        }

        if Self::is_cancelled(cancel_token) {
            return Err("已取消".into());
        }

        let manager_ctx = {
            let bb = blackboard.read().await;
            let mut ctx = String::new();
            let _ = write!(ctx, "角色: {RISK_MANAGER_ID}\n\n");
            for (i, report) in risk_reports.iter().enumerate() {
                let _ = write!(
                    ctx,
                    "--- {} 评估报告 ---\n{report}\n\n",
                    RISK_IDS.get(i).unwrap_or(&"unknown"),
                );
            }
            if let Some(debate) = bb.get_state("debate.summary") {
                let _ = write!(ctx, "\n--- 多空辩论摘要 ---\n{debate}\n");
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

    async fn phase_4b_trader(
        runner: &Option<Arc<dyn AgentRunner>>,
        blackboard: &Arc<RwLock<SharedBlackboard>>,
        events: &tokio::sync::broadcast::Sender<AnalysisEvent>,
        prompts: &Arc<HashMap<String, String>>,
        cancel_token: &Option<Arc<AtomicBool>>,
    ) -> Result<(), String> {
        if Self::is_cancelled(cancel_token) {
            return Err("已取消".into());
        }

        let _ = events.send(AnalysisEvent::AnalystProgress {
            expert_id: TRADER_ID.into(),
            status: "交易员制定执行方案...".into(),
            progress_pct: 85,
        });

        let trader_context = {
            let bb = blackboard.read().await;
            let mut ctx = String::new();
            let _ =
                write!(ctx, "角色: 交易员\n\n请基于以下分析结果制定具体的A股交易执行方案。\n\n");

            for analyst_id in ANALYST_IDS {
                let field = format!("report.{analyst_id}");
                if let Some(report) = bb.get_state(&field) {
                    let _ = write!(
                        ctx,
                        "--- {} 报告 ---\n{}\n\n",
                        analyst_id,
                        if report.len() > 500 {
                            &report[..500]
                        } else {
                            report
                        }
                    );
                }
            }

            if let Some(debate) = bb.get_state("debate.summary") {
                let _ = write!(ctx, "--- 多空辩论摘要 ---\n{debate}\n\n");
            }
            if let Some(risk) = bb.get_state("report.research-manager") {
                let _ = write!(
                    ctx,
                    "--- 研究经理综合评估 ---\n{}\n\n",
                    if risk.len() > 500 { &risk[..500] } else { risk }
                );
            }

            ctx
        };

        let sys_prompt = prompts
            .get(TRADER_ID)
            .cloned()
            .unwrap_or_else(|| fallback_prompt(TRADER_ID));

        let report = if let Some(ref r) = runner {
            r.run_agent(TRADER_ID, &sys_prompt, &trader_context).await?
        } else {
            Self::placeholder_analyst_report(TRADER_ID)
        };

        pipeline::write_report(TRADER_ID, &report, blackboard, events).await;

        let _ = events.send(AnalysisEvent::InvestmentPlan {
            plan: report.clone(),
        });

        Ok(())
    }

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

        let user_prompt = pipeline::build_analyst_context(PORTFOLIO_MANAGER_ID, blackboard).await;
        let sys_prompt = prompts::get_analyst_context(PORTFOLIO_MANAGER_ID, prompts)
            .unwrap_or_else(|| fallback_prompt(PORTFOLIO_MANAGER_ID));

        let decision_text = if let Some(ref r) = runner {
            r.run_agent(PORTFOLIO_MANAGER_ID, &sys_prompt, &user_prompt)
                .await?
        } else {
            r#"{"action":"持有","position_pct":0,"target_price":null,"stop_loss":null,"reasoning":"分析框架已完成，待 AgentRunner 注入后生成真实 LLM 决策。","risk_level":"中","confidence":0.0}"#.to_string()
        };

        let decision = Self::parse_decision(&decision_text).unwrap_or_else(|e| {
            tracing::warn!("决策解析失败，使用默认值: {e}");
            StockDecision {
                action: "持有".into(),
                position_pct: 0.0,
                target_price: None,
                stop_loss: None,
                reasoning: format!(
                    "解析失败: {e}。原始输出: {}",
                    &decision_text[..200.min(decision_text.len())]
                ),
                risk_level: "中".into(),
                confidence: 0.0,
            }
        });

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

    fn parse_decision(text: &str) -> Result<StockDecision, String> {
        if let Ok(d) = serde_json::from_str::<StockDecision>(text) {
            return Ok(d);
        }

        if let Some(start) = text.find("```json") {
            let inner = &text[start + 7..];
            if let Some(end) = inner.find("```") {
                let json_str = &inner[..end].trim();
                if let Ok(d) = serde_json::from_str::<StockDecision>(json_str) {
                    return Ok(d);
                }
            }
        }

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
