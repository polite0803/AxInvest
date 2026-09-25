// SPDX-License-Identifier: AGPL-3.0-only

//! 审批规则命令（PLAN-codex-parity R2-1）。
//!
//! 规则由 Bash 工具在用户批准后自动沉淀（best-effort），本模块只提供
//! 「查看」与「撤销」两个入口 —— 规则是用户显式批准的结果，故不由前端手工新增。
//!
//! 存储取全局注入的 [`axagent_harness::ApprovalRuleStore`]（`init/state.rs` 在启动时
//! 用 sea_orm 实现注入）；`None` 表示启动装配未完成 —— 此时列规则返回空，撤销报错。

use axagent_harness::ApprovalRuleStore;

/// 列出已沉淀的审批规则（供设置页展示 / 撤销）。
///
/// 存储未初始化时返回空列表（与「无规则」行为一致，不阻塞 UI 渲染）。
#[tauri::command]
pub async fn list_approval_rules() -> Result<Vec<axagent_harness::ApprovalRule>, String> {
    let Some(store) = axagent_tools::registry::global_approval_rule_store() else {
        return Ok(Vec::new());
    };
    Ok(store.list().await)
}

/// 撤销一条审批规则（按 `(program, args_prefix)` 精确匹配）。
///
/// 前端传参须用 camelCase（Tauri v2 命令参数默认 `rename_all = "camelCase"`）：
/// `invoke("revoke_approval_rule", { program, argsPrefix })`。
#[tauri::command]
pub async fn revoke_approval_rule(program: String, args_prefix: Vec<String>) -> Result<(), String> {
    let Some(store) = axagent_tools::registry::global_approval_rule_store() else {
        return Err(crate::commands::error::ErrorResponse::err(
            crate::commands::error_code::approval::RULE_STORE_UNAVAILABLE,
        ));
    };
    store.revoke(&program, &args_prefix).await.map_err(|e| {
        crate::commands::error::ErrorResponse::err_with_detail(
            crate::commands::error_code::approval::REVOKE_FAILED,
            e,
        )
    })
}
