// SPDX-License-Identifier: AGPL-3.0-only

//! Guardian 审查闸门的 wiring 实现（PLAN-codex-parity-adoption R3-2 接线）。
//!
//! 闸门本体在 consumer crate `axagent-agent`（`guardian` 模块：契约 + fail-closed 矩阵 +
//! 预算），审批发生在 hybrid crate `axagent-tools` —— 按 harness 依赖铁律 tools 不得依赖
//! agent，故两侧只能经 [`axagent_harness::GuardianBridge`] trait 连接。本文件是该 trait 的
//! **唯一**实现与唯一的安装点。
//!
//! ## 启用条件
//!
//! `settings.guardian_review_enabled`（默认 **false**）。开启后由
//! [`refresh_global_guardian_bridge`] 把桥接装进 `tools` 的全局槽，任何 `ToolRegistry`
//! 构建 `ToolContext` 时自动取用；保存设置时同步刷新，无需重启。
//!
//! 刻意**不做**的一件事：开了开关但没有可用 provider 时，本模块把全局槽置 `None`
//! 而非装入一个「必定拒绝」的桥。理由见 [`axagent_harness::GuardianBridge`] 文档 ——
//! fail-closed 针对的是「配了审查者却给不出结论」，不是「审查者压根没配」；
//! 后者若也拒，等于让一个未完成的 provider 配置瘫痪全部 Bash 审批。

use async_trait::async_trait;
use axagent_agent::guardian::{GuardianAction, GuardianConfig, GuardianScope, review_action};
use axagent_agent::llm_bridge::ProviderLlmBridge;
use axagent_harness::{AccessDecision, GuardianBridge};

/// 用主对话同款 provider 编排做审查者的闸门桥接。
///
/// 审查者模型 = `build_llm_bridge_from_db` 选出的「首个启用 provider 的首个启用模型」，
/// 不与被审的对话模型解耦。codex 用的是独立 reviewer 模型；本项目要独立配审查模型
/// 需先有「按用途选 provider」的通路，属独立改动面（见计划 §4.2「明确不做」）。
pub struct AppGuardianBridge {
    reviewer: ProviderLlmBridge,
}

impl AppGuardianBridge {
    pub fn new(reviewer: ProviderLlmBridge) -> Self {
        Self { reviewer }
    }
}

/// 手写 `Debug`：审查者持有 provider 句柄（含 api key 上下文），不进 Debug 输出。
/// 闸门桥要塞进 `ToolContext`（它 `derive(Debug)`），故必须实现。
impl std::fmt::Debug for AppGuardianBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppGuardianBridge").finish_non_exhaustive()
    }
}

#[async_trait]
impl GuardianBridge for AppGuardianBridge {
    async fn review(&self, payload: serde_json::Value, reason: Option<String>) -> AccessDecision {
        // require_guardian = false：输入超预算时闸门降级为「转人工确认」，
        // 而不是硬拒 —— 与既有「问用户」路径同向，且是 guardian 矩阵里唯一的问用户分支。
        let action = GuardianAction {
            scope: GuardianScope::Shell,
            payload,
            approval_reason: reason,
            require_guardian: false,
        };
        review_action(&GuardianConfig::default(), Some(&self.reviewer), &action).await
    }
}

/// 按当前 Settings 刷新全局审查闸门：安装 / 换人 / 停用。
///
/// 调用点：应用启动（`start_background_services`）与 `save_settings`。两处都持
/// `AppState`，都能拿到 DB 句柄与 master key，故共用本函数保证口径一致。
pub async fn refresh_global_guardian_bridge(state: &crate::app_state::AppState) {
    let db = state.harness.db().clone();
    let settings = match axagent_dao::repo::settings::get_settings(&db).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "[guardian] 读取 Settings 失败，审查闸门保持停用");
            axagent_tools::registry::set_global_guardian_bridge(None);
            return;
        },
    };
    if !settings.guardian_review_enabled {
        axagent_tools::registry::set_global_guardian_bridge(None);
        return;
    }
    let master_key = state.harness.master_key_owned();
    match axagent_runtime::llm_bridge::build_llm_bridge_from_db(&master_key).await {
        Some(reviewer) => {
            axagent_tools::registry::set_global_guardian_bridge(Some(std::sync::Arc::new(
                AppGuardianBridge::new(reviewer),
            )));
            tracing::info!("[guardian] 审查闸门已启用（Bash 审批先过 LLM 二次判定）");
        },
        None => {
            axagent_tools::registry::set_global_guardian_bridge(None);
            tracing::warn!(
                "[guardian] 开关已开但无可用 provider —— 审查闸门停用，Bash 审批回到问用户"
            );
        },
    }
}
