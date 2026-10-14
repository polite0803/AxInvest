// SPDX-License-Identifier: AGPL-3.0-only

//! 统一的命令层错误类型
//!
//! 后端返回结构化错误（错误码 + 分类），前端根据 `category` 做分支处理：
//! - `retryable`: 可自动重试
//! - `permission_denied`: 引导用户授权
//! - `unrecoverable`: 显示错误并停止
//! - `validation`: 提示用户修正输入
//!
//! 使用方式(示意伪代码，非可编译片段):
//! ```ignore
//! use crate::commands::error::{CommandError, ErrorCategory};
//!
//! // 简单错误
//! return Err(CommandError::new(error_code::conversation::NOT_FOUND));
//!
//! // 带分类 + 详情的错误
//! return Err(CommandError::new(error_code::tool::EXECUTION_TIMEOUT)
//!     .with_category(crate::commands::error::ErrorCategory::Retryable)
//!     .with_detail("Tool execution timed out after 30s".to_string()));
//!
//! // 从已有错误转换（替代 .map_err(|e| String::from(crate::commands::error::ErrorResponse::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable)))）
//! some_op().map_err(|e| CommandError::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable))?;
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::string::ToString;

/// 错误分类，供前端做分支处理（重试 / 授权 / 放弃等）。
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    /// 可重试错误：网络超时、临时故障、资源暂时不可用等
    Retryable,
    /// 权限拒绝：未经授权访问资源，需引导用户授权
    PermissionDenied,
    /// 不可恢复错误：数据损坏、内部状态不一致、前置条件永久不满足
    Unrecoverable,
    /// 输入验证错误：参数缺失、格式不正确、值域不合法
    Validation,
    /// 通用错误（默认分类）
    #[default]
    General,
}

/// 统一错误响应结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    /// 错误码，用于前端 i18n 翻译查询
    pub code: String,

    /// 错误分类，供前端做分支处理
    #[serde(default)]
    pub category: ErrorCategory,

    /// 技术详情，用于调试和日志记录
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,

    /// 翻译参数，用于替换错误消息中的占位符
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<HashMap<String, String>>,
}

/// 命令层统一错误类型（C1+M5）。
///
/// 替代全局的 `Result<T, String>` 模式，为前端提供可编程的错误分类与错误码。
/// 实现了 `Serialize` + `Display`（输出 JSON），可直接作为 Tauri 命令的 `Err` 类型。
pub type CommandError = ErrorResponse;

impl ErrorResponse {
    /// 创建新的错误响应（默认分类为 General）
    pub fn new(code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            category: crate::commands::error::ErrorCategory::General,
            detail: None,
            params: None,
        }
    }

    /// 创建带分类的错误响应
    pub fn with_category(mut self, category: ErrorCategory) -> Self {
        self.category = category;
        self
    }

    pub fn err(code: impl Into<String>) -> String {
        Self::new(code).to_string()
    }

    pub fn err_with_detail(code: impl Into<String>, detail: impl Into<String>) -> String {
        Self::new(code).with_detail(detail).to_string()
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// 添加翻译参数
    pub fn with_params(mut self, params: HashMap<String, String>) -> Self {
        self.params = Some(params);
        self
    }

    /// 添加单个翻译参数
    pub fn with_param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        let key = key.into();
        let value = value.into();

        match self.params {
            Some(ref mut params) => {
                params.insert(key, value);
            },
            None => {
                let mut params = HashMap::new();
                params.insert(key, value);
                self.params = Some(params);
            },
        }
        self
    }

    /// 从任意可 Display 的错误创建 CommandError。
    ///
    /// 用于替换 `.map_err(|e| String::from(crate::commands::error::ErrorResponse::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable)))` 模式：
    /// ```ignore
    /// some_op().map_err(|e| CommandError::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable))?;
    /// ```
    pub fn from_error(e: impl std::fmt::Display, category: ErrorCategory) -> Self {
        Self {
            code: "COMMON_INTERNAL".to_string(),
            category,
            detail: Some(e.to_string()),
            params: None,
        }
    }

    /// 从错误码 + Display 错误创建（保留原始错误码）。
    pub fn from_error_with_code(
        code: impl Into<String>,
        e: impl std::fmt::Display,
        category: ErrorCategory,
    ) -> Self {
        Self { code: code.into(), category, detail: Some(e.to_string()), params: None }
    }
}

/// 从 String 转换为 ErrorResponse（兼容旧代码，分类为 General）
impl From<String> for ErrorResponse {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

/// 将 ErrorResponse 转换为 String，使 `?` 运算符和 `.into()` 可以直接使用
impl From<ErrorResponse> for String {
    fn from(e: ErrorResponse) -> Self {
        e.to_string()
    }
}

/// 从 &str 转换为 ErrorResponse
impl From<&str> for ErrorResponse {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

/// 从 (String, String) 元组转换为 ErrorResponse
/// 元组格式: (code, detail)
impl From<(String, String)> for ErrorResponse {
    fn from((code, detail): (String, String)) -> Self {
        Self::new(code).with_detail(detail)
    }
}

/// 从 (&str, &str) 元组转换为 ErrorResponse
impl From<(&str, &str)> for ErrorResponse {
    fn from((code, detail): (&str, &str)) -> Self {
        Self::new(code).with_detail(detail)
    }
}

/// 将 ErrorResponse 转换为 JSON 字符串
impl std::fmt::Display for ErrorResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let json = serde_json::to_string(self).unwrap_or_else(|_| {
            format!(
                r#"{{"code":"{}","category":"{}","detail":{}}}"#,
                self.code,
                serde_json::to_string(&self.category).unwrap_or_else(|_| "\"general\"".into()),
                self.detail
                    .as_ref()
                    .map(|d| format!(r#""{}""#, d))
                    .unwrap_or_else(|| "null".to_string())
            )
        });
        write!(f, "{}", json)
    }
}

// SECURITY (C9): 实现 std::error::Error trait，使 ErrorResponse 可用于 anyhow::Result
// 和 `?` 操作符的错误链传播。
impl std::error::Error for ErrorResponse {}

/// 把命令层错误串压成**可以安全嵌进自然语言摘要**的一句技术详情。
///
/// 为什么需要：命令层按错误码规范返回的是 `ErrorResponse` 的 JSON 串，前端
/// `translateBackendError` 只在「整串就是 JSON」时才解析（`src/lib/errorI18n.ts` 的
/// `parseBackendError` 判 `startsWith("{") && endsWith("}")`）。定时任务把错误拼进
/// `TaskRunResult.output` 的摘要散文时，JSON 会原样留在任务中心那一行里 —— 既读不通、
/// 也翻不了（2026-10-14 实测形态）。所以落笔前先取 `detail`；没有 `detail` 就取 `code`
/// （至少是可辨识的码串，不是整段 JSON）；**根本不是 JSON 的原文原样返回**（零回归）。
pub fn error_text_for_prose(err: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(err) {
        Ok(v) => {
            let detail = v.get("detail").and_then(|d| d.as_str()).unwrap_or("");
            if !detail.is_empty() {
                return detail.to_string();
            }
            match v.get("code").and_then(|c| c.as_str()) {
                Some(code) => code.to_string(),
                None => err.to_string(),
            }
        },
        Err(_) => err.to_string(),
    }
}

#[cfg(test)]
mod error_text_for_prose_tests {
    use super::error_text_for_prose;

    #[test]
    fn json_带_detail_时只取_detail() {
        let s = r#"{"code":"STOCK_WORKFLOW_SERIALIZE_FAILED","category":"general","detail":"rules_json 序列化失败: io"}"#;
        assert_eq!(error_text_for_prose(s), "rules_json 序列化失败: io");
    }

    #[test]
    fn json_无_detail_时退到_code_不许漏出整段_json() {
        let s = r#"{"code":"CONVERSATION_NOT_FOUND","category":"general","detail":null}"#;
        assert_eq!(error_text_for_prose(s), "CONVERSATION_NOT_FOUND");
    }

    #[test]
    fn 非_json_原文透传_零回归() {
        assert_eq!(error_text_for_prose("no such table: x"), "no such table: x");
    }

    #[test]
    fn 裸数字串_不当成结构化错误() {
        // serde_json 会把 "5" 解析成数字 ⇒ 既无 detail 也无 code ⇒ 必须原样返回
        assert_eq!(error_text_for_prose("5"), "5");
    }
}
