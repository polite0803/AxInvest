// SPDX-License-Identifier: AGPL-3.0-only

//! Bot 审批级别解析（docs/bot-integration/P2/ADR-P2-1 / ADR-P2-4）。
//!
//! 纯函数 + 两张静态表，无 IO：解释权集中于此，避免散落在 `agent_query` 与
//! cron executor 两处。`bots.approval_mode` 仍只存不解释（ADR-007）。
//!
//! 规则（按 OpenMausBot 原义，修正 P1 GAP §2.3 的 max 转述）：
//! 1. 会话档位非 `default` 时直接覆盖 Bot 级别（specific 覆盖 general，双向）；
//!    `default`/未设时用 Bot 级别。
//! 2. 非交互源（例程）只取 Bot 级别，不读会话档位（例程会话不复用交互会话）。
//! 3. `custom` 不参与映射，落 `Prompt`（最安全解释），`level_supported` 全 provider
//!    返回 false。
//! 4. 只降不升护栏保留：函数结果仍流经 `resolve_effective_permission`（task_shape）。

use axagent_harness::runtime_types::permissions::PermissionMode;

/// OMB 五档 -> AxAgent PermissionMode（ADR-P2-4）。
///
/// `auto` 落 `WorkspaceWrite`（开放问题 Q1 的保守建议值；改一行可切
/// `DangerFullAccess`）。`custom` 落 `Prompt` 且不可保存（见 `level_supported`）。
fn approval_mode_to_permission(mode: &str) -> PermissionMode {
    match mode {
        "edits" | "auto" => PermissionMode::WorkspaceWrite,
        "full" => PermissionMode::Allow,
        // ask / custom / 未知值（DB 校验外的历史脏数据）：最安全解释
        _ => PermissionMode::Prompt,
    }
}

/// 引擎（provider）是否支持某 Bot 档位（ADR-P2-4 / GAP G-18）。
///
/// AxAgent 全部是 API 引擎：档位每轮读 `PermissionPolicy`，无引擎级限制；
/// 唯一例外是 `custom`（无静态映射语义），对所有 provider 返回 false。
pub fn level_supported(_provider: &str, approval_mode: &str) -> bool {
    approval_mode != "custom"
}

/// 解析 Bot 会话的生效 PermissionMode（ADR-P2-1 第 1 条）。
///
/// `session_mode` 是会话档位（`default` / `accept_edits` / `full_access`）：
/// 非 `default` 直接覆盖 Bot 级别（specific 覆盖 general，双向升降）；
/// `default`/未设时用 Bot 级别。
pub fn resolve_bot_approval_level(
    bot_approval_mode: &str,
    session_mode: Option<&str>,
) -> PermissionMode {
    match session_mode {
        Some("full_access") => PermissionMode::Allow,
        Some("accept_edits") => PermissionMode::WorkspaceWrite,
        // default / 未设：用 Bot 级别
        Some("default") | None => approval_mode_to_permission(bot_approval_mode),
        // 未知会话档位：静默回落 Prompt（与既有三档映射的未知值口径一致，GAP G-13）
        Some(_) => PermissionMode::Prompt,
    }
}

/// 非交互源（例程）的生效 PermissionMode（ADR-P2-1 第 2 条）：只取 Bot 档，
/// 不读会话档位。custom 同样落 Prompt。
pub fn resolve_bot_routine_level(bot_approval_mode: &str) -> PermissionMode {
    approval_mode_to_permission(bot_approval_mode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axagent_harness::runtime_types::permissions::PermissionMode;

    // ── 五档映射表全量（ADR-P2-4）──
    #[test]
    fn five_mode_mapping_table() {
        assert_eq!(resolve_bot_routine_level("ask"), PermissionMode::Prompt);
        assert_eq!(resolve_bot_routine_level("edits"), PermissionMode::WorkspaceWrite);
        // Q1 建议值：auto -> WorkspaceWrite
        assert_eq!(resolve_bot_routine_level("auto"), PermissionMode::WorkspaceWrite);
        assert_eq!(resolve_bot_routine_level("full"), PermissionMode::Allow);
        // custom 不参与映射：最安全解释
        assert_eq!(resolve_bot_routine_level("custom"), PermissionMode::Prompt);
    }

    // ── level_supported：custom 对所有 provider false，其余全支持 ──
    #[test]
    fn level_supported_custom_false_others_true() {
        for provider in ["openai", "anthropic", "gemini", "openclaw", "ollama"] {
            assert!(!level_supported(provider, "custom"), "{provider} 须拒绝 custom");
            for mode in ["ask", "edits", "auto", "full"] {
                assert!(level_supported(provider, mode), "{provider} 须支持 {mode}");
            }
        }
    }

    // ── 会话覆盖双向（ADR-P2-1 第 1 条）──
    #[test]
    fn session_override_bidirectional() {
        // 会话档位非 default 时覆盖 Bot 级别：升
        assert_eq!(resolve_bot_approval_level("ask", Some("full_access")), PermissionMode::Allow);
        // 降
        assert_eq!(
            resolve_bot_approval_level("full", Some("accept_edits")),
            PermissionMode::WorkspaceWrite
        );
        // default / 未设：用 Bot 级别
        assert_eq!(
            resolve_bot_approval_level("edits", Some("default")),
            PermissionMode::WorkspaceWrite
        );
        assert_eq!(resolve_bot_approval_level("full", None), PermissionMode::Allow);
        // 未知会话档位：静默回落 Prompt（与既有三档映射的未知值口径一致）
        assert_eq!(
            resolve_bot_approval_level("full", Some("unknown_mode")),
            PermissionMode::Prompt
        );
    }

    // ── 例程路径只取 Bot 档，不读会话档位 ──
    #[test]
    fn routine_path_ignores_session_mode() {
        // 例程对任意「假想会话档位」都返回同一 Bot 档结果（函数不接收会话参数即是约束）；
        // 与交互路径的差异点：非 default 会话档位在交互路径会覆盖 Bot 档，例程不会。
        for mode in ["ask", "edits", "auto", "full", "custom"] {
            let routine = resolve_bot_routine_level(mode);
            // default/None 会话：交互路径与例程一致
            assert_eq!(resolve_bot_approval_level(mode, None), routine);
            assert_eq!(resolve_bot_approval_level(mode, Some("default")), routine);
            // 非 default 会话：交互路径被会话覆盖，例程仍取 Bot 档（两者必然不同，除非恰好同值）
            assert_eq!(
                resolve_bot_routine_level(mode),
                routine,
                "例程结果与会话档位无关（纯函数无会话参数）"
            );
        }
    }
}
