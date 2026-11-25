// SPDX-License-Identifier: AGPL-3.0-only

//! P5 费用 / Token 配额解析与熔断（docs/bot-integration/P5/ADR-P5-2/3/4、MODEL-P5 §2/§3）。
//!
//! 作用域：Bot 自身（scope='bot'）与部门合计（scope='department'，按 `bot_usage_ledger.department_id`
//! 写入时快照聚合）。无部门归属的 Bot 只受自身配额约束。
//!
//! 熔断时机：**新建 run / 新建轮前**。已启动的运行继续跑完；交互轮的追问不被中途打断
//! （ADR-P5-3）。

use axagent_dao::DatabaseConnection;
use axagent_dao::repo::bot as bot_repo;
use axagent_dao::repo::bot_cost_quota as quota_repo;
use axagent_dao::repo::bot_department as dept_repo;
use axagent_dao::repo::bot_usage_ledger as ledger_repo;
use axagent_harness::core_error::AxAgentError;
use axagent_harness::conversation_model::TokenUsage;
use axagent_harness::types::{
    BotCostQuotaInput, BotCostQuotaScope, BotUsageEntry, BotUsageFilter, BotUsageOrigin,
    BotUsageSummary, BotPricingSource,
};
use axagent_harness::usage_pricing::{ModelPricing, pricing_for_model};

use crate::commands::error::{ErrorCategory, ErrorResponse};
use crate::commands::error_code::bot as bot_err;

/// 默认窗口：7 天（168 小时），与 ADR-P5-2 / MODEL-P5 一致。
pub const DEFAULT_WINDOW_HOURS: i64 = 168;

/// 熔断检查结果。`breached=false` 表示未超限或无配额。
#[derive(Debug, Clone, PartialEq)]
pub struct CostQuotaCheck {
    pub breached: bool,
    pub scope: &'static str,
    pub scope_id: String,
    pub window_hours: i64,
    pub used_total_tokens: i64,
    pub used_cost_usd: f64,
    pub max_total_tokens: i64,
    pub max_cost_usd: f64,
}

impl CostQuotaCheck {
    pub fn ok(scope: &'static str, scope_id: impl Into<String>, window_hours: i64) -> Self {
        Self {
            breached: false,
            scope,
            scope_id: scope_id.into(),
            window_hours,
            used_total_tokens: 0,
            used_cost_usd: 0.0,
            max_total_tokens: 0,
            max_cost_usd: 0.0,
        }
    }
}

fn quota_invalid(field: &str, detail: impl Into<String>) -> String {
    ErrorResponse::new(bot_err::COST_QUOTA_INVALID)
        .with_category(ErrorCategory::Validation)
        .with_param("field", field)
        .with_detail(detail)
        .to_string()
}

/// `BOT_COST_QUOTA_EXCEEDED` 的标准错误出口。
pub fn quota_breached_response(check: &CostQuotaCheck) -> String {
    ErrorResponse::new(bot_err::COST_QUOTA_EXCEEDED)
        .with_category(ErrorCategory::Validation)
        .with_param("scope", check.scope)
        .with_param("scopeId", &check.scope_id)
        .with_param("usedTotalTokens", check.used_total_tokens.to_string())
        .with_param("usedCostUsd", format!("{:.6}", check.used_cost_usd))
        .with_param("maxTotalTokens", check.max_total_tokens.to_string())
        .with_param("maxCostUsd", format!("{:.6}", check.max_cost_usd))
        .with_detail(format!(
            "{}:{} 窗口 {}h 内已用 tokens={} cost_usd={:.6}，上限 tokens={} cost_usd={:.6}",
            check.scope,
            check.scope_id,
            check.window_hours,
            check.used_total_tokens,
            check.used_cost_usd,
            check.max_total_tokens,
            check.max_cost_usd
        ))
        .to_string()
}

/// scope 合法性校验。
pub fn validate_scope(scope: &str) -> Result<(), String> {
    match scope {
        "bot" | "department" => Ok(()),
        other => Err(quota_invalid(
            "scope",
            format!("scope 必须是 bot/department，实得 {other}"),
        )),
    }
}

/// 输入校验：上限非负；window 合理。全 0 = 不限，由命令层归一为删行。
pub fn validate_quota_input(input: &BotCostQuotaInput) -> Result<(), String> {
    validate_scope(input.scope.as_str())?;
    if input.max_total_tokens < 0 {
        return Err(quota_invalid("maxTotalTokens", "maxTotalTokens 不能为负数"));
    }
    if input.max_cost_usd < 0.0 {
        return Err(quota_invalid("maxCostUsd", "maxCostUsd 不能为负数"));
    }
    if input.window_hours <= 0 || input.window_hours > 24 * 365 {
        return Err(quota_invalid("windowHours", "windowHours 必须在 1..8760 之间"));
    }
    Ok(())
}

/// 窗口起点（epoch millis）。
pub fn window_since_ms(window_hours: i64) -> i64 {
    axagent_harness::util_fns::now_ms() - window_hours.max(1) * 3_600_000
}

/// 对单个 scope 检查是否超限。
async fn check_one_scope(
    db: &DatabaseConnection,
    scope: &'static str,
    scope_id: &str,
) -> Result<CostQuotaCheck, AxAgentError> {
    let Some(quota) = quota_repo::get_by_scope(db, scope, scope_id).await? else {
        return Ok(CostQuotaCheck::ok(scope, scope_id, DEFAULT_WINDOW_HOURS));
    };
    // 两上限都为 0 视为不限（防御：正常写入路径已归一为删行）。
    if quota.max_total_tokens <= 0 && quota.max_cost_usd <= 0.0 {
        return Ok(CostQuotaCheck::ok(scope, scope_id, quota.window_hours as i64));
    }
    let since = window_since_ms(quota.window_hours as i64);
    let totals = match scope {
        "bot" => ledger_repo::sum_by_bot(db, scope_id, since, i64::MAX).await?,
        _ => ledger_repo::sum_by_department(db, scope_id, since, i64::MAX).await?,
    };
    let breached = (quota.max_total_tokens > 0 && totals.total_tokens >= quota.max_total_tokens)
        || (quota.max_cost_usd > 0.0 && totals.estimated_cost_usd >= quota.max_cost_usd);
    Ok(CostQuotaCheck {
        breached,
        scope,
        scope_id: scope_id.to_string(),
        window_hours: quota.window_hours as i64,
        used_total_tokens: totals.total_tokens,
        used_cost_usd: totals.estimated_cost_usd,
        max_total_tokens: quota.max_total_tokens,
        max_cost_usd: quota.max_cost_usd,
    })
}

/// 熔断检查：先查 Bot 自身配额，再查部门配额（若 Bot 归属部门）。任一超限即 breached。
pub async fn check_bot_cost_quota(
    db: &DatabaseConnection,
    bot_id: &str,
) -> Result<CostQuotaCheck, AxAgentError> {
    let bot_check = check_one_scope(db, "bot", bot_id).await?;
    if bot_check.breached {
        return Ok(bot_check);
    }
    if let Some(dept_id) = dept_repo::department_of_bot(db, bot_id).await? {
        let dept_check = check_one_scope(db, "department", &dept_id).await?;
        if dept_check.breached {
            return Ok(dept_check);
        }
    }
    Ok(bot_check)
}

/// 熔断检查（命令/runner 入口用）：超限返回 `BOT_COST_QUOTA_EXCEEDED`。
pub async fn ensure_bot_cost_quota_ok(db: &DatabaseConnection, bot_id: &str) -> Result<(), String> {
    let check = check_bot_cost_quota(db, bot_id).await.map_err(|e| {
        ErrorResponse::from_error(e, ErrorCategory::Unrecoverable).to_string()
    })?;
    if check.breached {
        return Err(quota_breached_response(&check));
    }
    Ok(())
}

/// 按模型名解析计价档位与来源（ADR-P5-4：四类 token 计价，来源随行快照）。
pub fn pricing_for(model_id: &str) -> (ModelPricing, BotPricingSource) {
    match pricing_for_model(model_id) {
        Some(p) => (p, BotPricingSource::Model),
        None => (ModelPricing::default_sonnet_tier(), BotPricingSource::Heuristic),
    }
}

/// 记录一次 Bot 用量到账本（append-only）。
#[allow(clippy::too_many_arguments)]
pub async fn record_usage(
    db: &DatabaseConnection,
    bot_id: &str,
    conversation_id: Option<&str>,
    execution_id: Option<&str>,
    workflow_run_id: Option<&str>,
    assignment_id: Option<&str>,
    origin: BotUsageOrigin,
    model_id: &str,
    usage: &TokenUsage,
) -> Result<(), String> {
    let bot = bot_repo::get_bot(db, bot_id).await.map_err(|e| e.to_string())?;
    // department_id 快照：写入时取值（无部门则 None）。
    let department_id = dept_repo::department_of_bot(db, bot_id).await.ok().flatten();

    let (pricing, pricing_source) = pricing_for(model_id);
    let estimate = pricing.cost_for(*usage);

    let entry = BotUsageEntry {
        id: String::new(),
        bot_id: bot.id,
        department_id,
        conversation_id: conversation_id.map(str::to_string),
        execution_id: execution_id.map(str::to_string),
        workflow_run_id: workflow_run_id.map(str::to_string),
        assignment_id: assignment_id.map(str::to_string),
        model_id: model_id.to_string(),
        origin,
        input_tokens: usage.input_tokens as i64,
        output_tokens: usage.output_tokens as i64,
        cache_creation_tokens: usage.cache_creation_input_tokens as i64,
        cache_read_tokens: usage.cache_read_input_tokens as i64,
        total_tokens: usage.total_tokens() as i64,
        estimated_cost_usd: estimate.total_cost_usd(),
        pricing_source,
        created_at: 0,
    };
    ledger_repo::append(db, &entry).await.map_err(|e| e.to_string())?;
    Ok(())
}

/// 汇总查询（`get_bot_usage_summary`）。
pub async fn usage_summary(
    db: &DatabaseConnection,
    scope: &str,
    scope_id: &str,
    window_hours: Option<i64>,
) -> Result<BotUsageSummary, String> {
    validate_scope(scope)?;
    let quota =
        quota_repo::get_by_scope(db, scope, scope_id).await.map_err(|e| e.to_string())?;
    let wh = window_hours.unwrap_or_else(|| {
        quota
            .as_ref()
            .map(|q| q.window_hours as i64)
            .unwrap_or(DEFAULT_WINDOW_HOURS)
    });
    let since = window_since_ms(wh);
    let filter = match scope {
        "bot" => BotUsageFilter {
            bot_id: Some(scope_id.to_string()),
            since: Some(since),
            ..Default::default()
        },
        _ => BotUsageFilter {
            department_id: Some(scope_id.to_string()),
            since: Some(since),
            ..Default::default()
        },
    };
    let rows = ledger_repo::list(db, &filter).await.map_err(|e| e.to_string())?;
    let mut totals = axagent_harness::types::BotUsageTotals::default();
    for r in &rows {
        totals.input_tokens += r.input_tokens;
        totals.output_tokens += r.output_tokens;
        totals.cache_creation_tokens += r.cache_creation_tokens;
        totals.cache_read_tokens += r.cache_read_tokens;
        totals.total_tokens += r.total_tokens;
        totals.estimated_cost_usd += r.estimated_cost_usd;
    }
    totals.entry_count = rows.len() as i64;

    let (max_tokens, max_cost) = quota
        .as_ref()
        .map(|q| (q.max_total_tokens, q.max_cost_usd))
        .unwrap_or((0, 0.0));
    let breached = (max_tokens > 0 && totals.total_tokens >= max_tokens)
        || (max_cost > 0.0 && totals.estimated_cost_usd >= max_cost);

    Ok(BotUsageSummary {
        scope: if scope == "bot" {
            BotCostQuotaScope::Bot
        } else {
            BotCostQuotaScope::Department
        },
        scope_id: scope_id.to_string(),
        window_hours: wh as i32,
        input_tokens: totals.input_tokens,
        output_tokens: totals.output_tokens,
        cache_creation_tokens: totals.cache_creation_tokens,
        cache_read_tokens: totals.cache_read_tokens,
        total_tokens: totals.total_tokens,
        estimated_cost_usd: totals.estimated_cost_usd,
        entry_count: totals.entry_count,
        breached,
        pricing_note: "估算值：按本地价格表计算，与服务商账单可能存在差异".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> BotCostQuotaInput {
        BotCostQuotaInput {
            scope: BotCostQuotaScope::Bot,
            scope_id: "bot-1".into(),
            max_total_tokens: 1000,
            max_cost_usd: 1.0,
            window_hours: 168,
        }
    }

    #[test]
    fn scope_validation_rejects_unknown() {
        validate_scope("bot").expect("bot 合法");
        validate_scope("department").expect("department 合法");
        let err = validate_scope("team").expect_err("team 非法");
        assert!(err.contains("BOT_COST_QUOTA_INVALID"), "{err}");
        assert!(err.contains("scope"), "detail 带字段名: {err}");
    }

    #[test]
    fn input_validation_reports_each_field() {
        let mut i = input();
        i.max_total_tokens = -1;
        let err = validate_quota_input(&i).expect_err("负上限");
        assert!(err.contains("maxTotalTokens"), "{err}");

        let mut i = input();
        i.max_cost_usd = -0.5;
        let err = validate_quota_input(&i).expect_err("负费用");
        assert!(err.contains("maxCostUsd"), "{err}");

        let mut i = input();
        i.window_hours = 0;
        let err = validate_quota_input(&i).expect_err("窗口越界");
        assert!(err.contains("windowHours"), "{err}");

        validate_quota_input(&input()).expect("合法输入应通过");
    }

    #[test]
    fn window_since_is_roughly_the_window() {
        let now = axagent_harness::util_fns::now_ms();
        let since = window_since_ms(24);
        let delta = now - since;
        assert!((delta - 24 * 3_600_000).abs() < 5_000, "窗口起点应约 24h 前: {delta}");
    }

    #[test]
    fn breached_response_carries_scope_and_limits() {
        let check = CostQuotaCheck {
            breached: true,
            scope: "bot",
            scope_id: "bot-1".into(),
            window_hours: 168,
            used_total_tokens: 2000,
            used_cost_usd: 3.0,
            max_total_tokens: 1000,
            max_cost_usd: 1.0,
        };
        let raw = quota_breached_response(&check);
        assert!(raw.contains("BOT_COST_QUOTA_EXCEEDED"), "{raw}");
        assert!(raw.contains("bot-1"), "detail 带 scope id: {raw}");
        assert!(raw.contains("2000") && raw.contains("1000"), "detail 带用量与上限: {raw}");
    }

    #[test]
    fn pricing_source_reflects_model_hit() {
        let (_, src) = pricing_for("gpt-5.5");
        assert_eq!(src, BotPricingSource::Model);
        let (_, src) = pricing_for("totally-unknown-model-xyz");
        assert_eq!(src, BotPricingSource::Heuristic);
    }
}
