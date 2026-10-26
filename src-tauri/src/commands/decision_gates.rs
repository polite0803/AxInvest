// SPDX-License-Identifier: AGPL-3.0-only

//! 决策否决门开关的设置命令（2026-10-26 用户裁决）。
//!
//! ## 四门是什么
//!
//! | 门 | 判据 | 放开后的效果 |
//! |---|---|---|
//! | 风控门 | 风险档 = 高 / 极高 ⇒ 买入→持有 / 观望 | 高/极高风险票**可以**开仓（仓位仍按 `pm_risk_cap` 封顶） |
//! | 情绪门 | 全市场封板率 < 0.5 ⇒ 不出买入/增持 | 超短档**不因市场情绪**被否决 |
//! | 估值门 | DCF upside ≤ −35% 或 PE ≥ 自身 5 年 80 分位 ⇒ 不出买入/增持 | 长档**不因严重高估**被否决 |
//! | 顶层采纳档内买入 | `R-CALIB-CAP`（校准未达标 ⇒ 置信封顶 ⇒ 动作联动降级） | 顶层**采纳**档内已判出的「买入」 |
//!
//! ⚠ 「数据缺席」（如估值门 `unjudged` = DCF 不适用且 PE 分位缺失）**不是门**，
//! 放开高估门**不会**把它变成「通过」—— 缺席 ≠ 通过是数据契约，不是产品策略。
//!
//! ⚠ 放开**只停止否决**，**不抬信号**：门从来单向拦；放开后动作仍由评分/后验决定。
//!
//! ## 状态来源与落库
//!
//! * 定义唯一权威 = `settings` 表（key = `decision_gates::Gate::key()`）。
//! * 进程内快照供 Rhai 读（宿主函数是同步闭包，不能在脚本里 await DB）。
//! * 本模块的写命令 = **写 DB + 立即刷新内存** ⇒ 下一轮分析即生效，**不必重启**、**不必换代**。

use crate::AppState;
use crate::commands::error::ErrorResponse;
use crate::commands::error_code::storage as storage_err;
use axagent_agent_macro::agent_command;
use axagent_analysis_engine::decision_gates::{self, Gate};
use serde::Serialize;
use tauri::Manager;

/// 四门状态（回显用）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionGatesState {
    /// 风控门（高/极高风险禁止加仓/持仓）是否已放开。
    pub risk_veto_enabled: bool,
    /// 情绪门（全市场封板率）是否已放开。
    pub sentiment_enabled: bool,
    /// 估值门（严重高估）是否已放开。
    pub valuation_enabled: bool,
    /// 顶层是否采纳档内买入（解除 `R-CALIB-CAP` 的动作联动）。
    pub tier_buy_enabled: bool,
}

impl DecisionGatesState {
    fn from_flags(flags: [bool; 4]) -> Self {
        Self {
            risk_veto_enabled: flags[0],
            sentiment_enabled: flags[1],
            valuation_enabled: flags[2],
            tier_buy_enabled: flags[3],
        }
    }
}

/// 读当前四门状态（**以 DB 为准**，不是内存快照）。
///
/// ⚠ 读 DB 而非内存：设置面板要显示的是**用户已保存**的值，
/// 而内存快照可能被别的路径改过（本次实现只有设置命令会改，但语义上要以 DB 为准）。
#[agent_command(domain = app_config, safety = Caution, call_mode = Manual, description = "读决策否决门开关状态")]
#[tauri::command]
pub async fn get_decision_gates(app: tauri::AppHandle) -> Result<DecisionGatesState, String> {
    let state = app.state::<AppState>();
    let db = state.harness.db();
    let flags = decision_gates::refresh_from_db(db).await.map_err(|e| {
        String::from(ErrorResponse::new(storage_err::READ_FILE_FAILED).with_detail(e))
    })?;
    Ok(DecisionGatesState::from_flags(flags))
}

/// 写单个门（写 DB + 立即生效）。
///
/// ⚠ 只认 `gate` 的**完整键**（`decision_gates::Gate::key()`）；
/// 未知键 ⇒ 报错（不静默忽略，否则前端拼错键名会以为「设置成功」）。
#[agent_command(domain = app_config, safety = Caution, call_mode = Manual, description = "设置单个决策否决门开关")]
#[tauri::command]
pub async fn set_decision_gate(
    app: tauri::AppHandle,
    gate: String,
    enabled: bool,
) -> Result<DecisionGatesState, String> {
    let state = app.state::<AppState>();
    let db = state.harness.db();
    let parsed = Gate::from_key(gate.trim()).ok_or_else(|| {
        String::from(ErrorResponse::new(storage_err::WRITE_FILE_FAILED).with_detail(format!(
            "未知的否决门键「{gate}」；合法键：{}",
            [
                Gate::RiskVeto.key(),
                Gate::Sentiment.key(),
                Gate::Valuation.key(),
                Gate::TierBuy.key(),
            ]
            .join(" / ")
        )))
    })?;
    decision_gates::persist_and_apply(db, parsed, enabled).await.map_err(|e| {
        String::from(ErrorResponse::new(storage_err::WRITE_FILE_FAILED).with_detail(e))
    })?;
    get_decision_gates(app).await
}

/// 一次性写四门（设置面板「保存」按钮）。
///
/// ⚠ 与逐个写**同一套落库逻辑**（`persist_and_apply`）⇒ 不会出现「批量写漏了一门」。
#[agent_command(domain = app_config, safety = Caution, call_mode = Manual, description = "批量设置决策否决门开关")]
#[tauri::command]
pub async fn set_decision_gates(
    app: tauri::AppHandle,
    risk_veto_enabled: bool,
    sentiment_enabled: bool,
    valuation_enabled: bool,
    tier_buy_enabled: bool,
) -> Result<DecisionGatesState, String> {
    let state = app.state::<AppState>();
    let db = state.harness.db();
    let pairs = [
        (Gate::RiskVeto, risk_veto_enabled),
        (Gate::Sentiment, sentiment_enabled),
        (Gate::Valuation, valuation_enabled),
        (Gate::TierBuy, tier_buy_enabled),
    ];
    for (gate, enabled) in pairs {
        decision_gates::persist_and_apply(db, gate, enabled).await.map_err(|e| {
            String::from(ErrorResponse::new(storage_err::WRITE_FILE_FAILED).with_detail(e))
        })?;
    }
    get_decision_gates(app).await
}
