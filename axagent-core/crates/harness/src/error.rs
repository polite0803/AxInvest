// SPDX-License-Identifier: AGPL-3.0-only

//! 工具错误类型 — 从 axagent-runtime-core 提取的契约接口
//!
//! `ToolError` 和 `ToolErrorKind` 是工具系统的核心错误类型，
//! 供 `axagent-runtime-core`、`axagent-tools`、`axagent-agent` 跨 crate 共享。
//! 各 crate 通过 `pub use axagent_harness::error::*` 重导出保持兼容。

use std::fmt::{Display, Formatter};

/// 工具调用失败时返回的错误
#[derive(Debug, Clone)]
pub struct ToolError {
    pub message: String,
    pub kind: ToolErrorKind,
    /// i18n 错误码，格式 "tool.{name}.{kind}" 或 "tool.{name}.{specific}"
    pub error_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolErrorKind {
    NotFound,
    PermissionDenied,
    /// 限流（调用过于频繁 / 超出时间窗口配额）。
    ///
    /// **为什么必须与 `PermissionDenied` 分开**（2026-09-21 实证）：此前限流被
    /// `ToolError::permission_denied` 复用，于是工具结果文本长这样 ——
    /// `[permissionDenied] 工具 'get_stock_margin_data' 权限被拒绝: 工具 '…' 调用过于频繁，
    /// 最小间隔 200ms（当前距上次调用 48ms）`。
    /// **模型据此把它读成「没权限」**并在分析报告里写成「工具调用被拒绝」，把使用者
    /// 引向错误的排查方向（真因只是两个并行节点撞了同一把 200ms 闸）。
    /// 限流是**可重试的时序约束**，权限拒绝是**不可重试的能力缺失**，二者语义相反。
    ///
    /// 该区分只影响 [`Display`] 前缀（即喂给 LLM 的工具结果文本）——
    /// `error_code` 字段与 `tool.common.*` 这组 i18n key 目前**两侧都无消费端**
    /// （见 `AUDIT-pledge-attribution-2026-09-21.md` §8.5），故本次**不**新增 i18n key，
    /// 避免制造新的死键。
    RateLimited,
    InvalidInput,
    ExecutionFailed,
    Timeout,
    Cancelled,
    RollbackNotSupported,
}

impl ToolError {
    fn kind_str(kind: &ToolErrorKind) -> &'static str {
        match kind {
            ToolErrorKind::NotFound => "notFound",
            ToolErrorKind::PermissionDenied => "permissionDenied",
            ToolErrorKind::RateLimited => "rateLimited",
            ToolErrorKind::InvalidInput => "invalidInput",
            ToolErrorKind::ExecutionFailed => "executionFailed",
            ToolErrorKind::Timeout => "timeout",
            ToolErrorKind::Cancelled => "cancelled",
            ToolErrorKind::RollbackNotSupported => "rollbackNotSupported",
        }
    }

    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ToolErrorKind::ExecutionFailed,
            error_code: String::new(),
        }
    }

    #[must_use]
    pub fn not_found(tool_name: &str) -> Self {
        Self {
            message: format!("工具 '{}' 未找到", tool_name),
            kind: ToolErrorKind::NotFound,
            error_code: format!("tool.{}.notFound", tool_name),
        }
    }

    #[must_use]
    pub fn permission_denied(tool_name: &str, reason: &str) -> Self {
        Self {
            message: format!("工具 '{tool_name}' 权限被拒绝: {reason}"),
            kind: ToolErrorKind::PermissionDenied,
            error_code: format!("tool.{tool_name}.permissionDenied"),
        }
    }

    /// 限流（调用过于频繁 / 超出窗口配额）。
    ///
    /// `reason` 由 `axagent_tools::audit::RateLimitViolation::message` 提供 ——
    /// 其中已含「距上次调用 N ms / 窗口上限 N 次」等**可操作**信息。
    ///
    /// 措辞刻意**不含**「权限 / 拒绝 / 未授权」等词：那类词会被模型读成能力缺失，
    /// 进而把一次可重试的时序冲突写成「工具调用被拒绝」（见 `ToolErrorKind::RateLimited`
    /// 的说明与 `AUDIT-pledge-attribution-2026-09-21.md`）。
    #[must_use]
    pub fn rate_limited(tool_name: &str, reason: &str) -> Self {
        Self {
            message: format!("工具 '{tool_name}' 触发调用频率限制: {reason}"),
            kind: ToolErrorKind::RateLimited,
            error_code: format!("tool.{tool_name}.rateLimited"),
        }
    }

    #[must_use]
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ToolErrorKind::InvalidInput,
            error_code: String::new(),
        }
    }

    #[must_use]
    pub fn invalid_input_for(tool_name: &str, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ToolErrorKind::InvalidInput,
            error_code: format!("tool.{tool_name}.invalidInput"),
        }
    }

    #[must_use]
    pub fn execution_failed(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ToolErrorKind::ExecutionFailed,
            error_code: String::new(),
        }
    }

    #[must_use]
    pub fn execution_failed_for(tool_name: &str, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ToolErrorKind::ExecutionFailed,
            error_code: format!("tool.{tool_name}.executionFailed"),
        }
    }

    #[must_use]
    pub fn timeout_for(tool_name: &str, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ToolErrorKind::Timeout,
            error_code: format!("tool.{tool_name}.timeout"),
        }
    }

    #[must_use]
    pub fn rollback_not_supported(tool_name: &str) -> Self {
        Self {
            message: format!("工具 '{tool_name}' 不支持回滚"),
            kind: ToolErrorKind::RollbackNotSupported,
            error_code: format!("tool.{tool_name}.rollbackNotSupported"),
        }
    }
}

impl Display for ToolError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", Self::kind_str(&self.kind), self.message)
    }
}

impl std::error::Error for ToolError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// P1-1 回归锁（2026-09-21）：限流必须用**独立**错误码，且文案**不得**含归因性表述。
    ///
    /// 背景实证：此前限流复用 `permission_denied`，工具结果文本是
    /// `[permissionDenied] 工具 'get_stock_margin_data' 权限被拒绝: … 调用过于频繁，
    ///  最小间隔 200ms（当前距上次调用 48ms）`
    /// ⇒ 模型把它读成「没权限」，在分析报告里写成「工具调用被拒绝」。
    /// 本测试把这条契约锁在**错误码 + Display 前缀 + 文案**三层上。
    #[test]
    fn rate_limited_is_distinct_from_permission_denied() {
        let err = ToolError::rate_limited("get_stock_margin_data", "调用过于频繁，最小间隔 200ms");
        assert_eq!(err.kind, ToolErrorKind::RateLimited);
        assert_ne!(err.kind, ToolErrorKind::PermissionDenied, "限流不得复用权限拒绝的 kind");
        assert_eq!(err.error_code, "tool.get_stock_margin_data.rateLimited");

        let text = err.to_string();
        assert!(
            text.starts_with("[rateLimited] "),
            "Display 前缀须为 `[rateLimited] `（这是**喂给 LLM 的工具结果文本**，\
             P1-1 的全部意义所在）。实际：{text}"
        );
        assert!(!text.contains("permissionDenied"), "不得再冒充权限拒绝。实际：{text}");
        // 措辞锁：这些词会让模型把可重试的时序冲突读成不可重试的能力缺失。
        for banned in ["权限被拒绝", "权限不足", "未授权", "无权限"] {
            assert!(!text.contains(banned), "限流文案含归因性表述「{banned}」：{text}");
        }
    }

    /// 对照：`permission_denied` 的形态**保持原样**（本改动的爆炸半径）。
    #[test]
    fn permission_denied_is_unchanged() {
        let err = ToolError::permission_denied("FileWrite", "需要用户批准");
        assert_eq!(err.kind, ToolErrorKind::PermissionDenied);
        assert_eq!(err.error_code, "tool.FileWrite.permissionDenied");
        assert_eq!(err.to_string(), "[permissionDenied] 工具 'FileWrite' 权限被拒绝: 需要用户批准");
    }
}
