// SPDX-License-Identifier: AGPL-3.0-only

//! 工具调用审计系统
//!
//! 提供：
//! 1. 工具调用频率限制（Rate Limiting）
//! 2. 输入参数敏感信息过滤
//! 3. 输出内容敏感信息扫描
//! 4. 调用审计日志

use axagent_entities::audit_log;
use sea_orm::{ConnectionTrait, DatabaseConnection, EntityTrait, Schema, Set};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// 进程级审计库连接。
///
/// 为什么不挂在 `ToolAuditor` 实例上：`UnifiedToolRegistry::new()` 全仓有 13 处调用
/// （含 agent 循环 / fleet / plan / compress 等真实工具执行路径，另有 6 处在测试内），
/// 每个实例各持一份连接既浪费、又要求逐个调用方注入 —— 漏掉任何一处，
/// 该路径的审计就静默不落库，而这正是本模块此前的状态。
///
/// 形态与 `sandbox_policy` 一致：全局注册一次，之后任何位置（含临时 `new()` 出来的
/// registry）自动回退读取。
///
/// 未注册（`None`）时退化为纯内存审计且不报错，保证单测与未初始化 DB 的场景可运行。
static AUDIT_DB: OnceLock<DatabaseConnection> = OnceLock::new();

/// 注册审计库连接，并确保 `audit_log` 表存在（建表语句由实体生成，幂等）。
///
/// 主程序启动时调用一次；重复调用保留首个连接并返回 `Ok`。
pub async fn init_audit_db(db: DatabaseConnection) -> Result<(), String> {
    let mut stmt =
        Schema::new(db.get_database_backend()).create_table_from_entity(audit_log::Entity);
    stmt.if_not_exists();
    db.execute(&stmt).await.map_err(|e| format!("创建 audit_log 表失败: {e}"))?;
    // `set` 返回 Err 表示已有值 —— 保留首个连接，属幂等成功。
    let _ = AUDIT_DB.set(db);
    Ok(())
}

/// 审计条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// 时间戳
    pub timestamp: i64,
    /// 工具名称
    pub tool_name: String,
    /// 会话 ID
    pub conversation_id: Option<String>,
    /// 是否成功
    pub success: bool,
    /// 执行耗时 (ms)
    pub duration_ms: u64,
    /// 输出截断后的前 200 字符
    pub output_preview: String,
    /// 参数是否包含敏感信息
    pub has_sensitive_input: bool,
    /// 输出是否包含敏感信息
    pub has_sensitive_output: bool,
}

/// 频率限制状态
struct RateLimitState {
    last_call: Instant,
    call_count: u32,
    window_start: Instant,
}

/// 审计器配置
#[derive(Debug, Clone)]
pub struct AuditConfig {
    /// 全局最小调用间隔 (ms)
    pub min_interval_ms: u64,
    /// 时间窗口内最大调用次数
    pub max_calls_per_window: u32,
    /// 时间窗口长度 (秒)
    pub window_secs: u64,
    /// 是否启用敏感信息扫描
    pub scan_sensitive: bool,
    /// 最大保留日志条数（**内存**环形上限；落库的条数不受此限制）
    pub max_log_entries: usize,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            min_interval_ms: 200,
            max_calls_per_window: 30,
            window_secs: 10,
            scan_sensitive: true,
            max_log_entries: 500,
        }
    }
}

/// 工具调用审计器
pub struct ToolAuditor {
    config: AuditConfig,
    /// 每个工具独立的频率限制状态
    rate_limits: RwLock<HashMap<String, RateLimitState>>,
    /// 审计日志（内存，保留最近 `max_log_entries` 条）
    log: RwLock<Vec<AuditEntry>>,
}

impl ToolAuditor {
    /// 构造审计器。
    ///
    /// 持久化连接不在构造时传入：审计落库走进程级的 [`AUDIT_DB`]，由
    /// [`init_audit_db`] 在启动时注册一次。未注册时退化为纯内存审计。
    /// 这样全部 13 处 `UnifiedToolRegistry::new()` 调用点无需改动即可落库。
    pub fn new(config: AuditConfig) -> Self {
        Self { config, rate_limits: RwLock::new(HashMap::new()), log: RwLock::new(Vec::new()) }
    }

    /// 检查频率限制，返回 Ok 表示允许，Err 表示触发限制
    pub async fn check_rate_limit(&self, tool_name: &str) -> Result<(), String> {
        let mut limits = self.rate_limits.write().await;
        let now = Instant::now();
        let state = limits.entry(tool_name.to_string()).or_insert(RateLimitState {
            last_call: now - Duration::from_secs(60),
            call_count: 0,
            window_start: now,
        });

        // 检查最小间隔
        let elapsed = now.duration_since(state.last_call);
        if elapsed < Duration::from_millis(self.config.min_interval_ms) {
            return Err(format!(
                "工具 '{}' 调用过于频繁，最小间隔 {}ms（当前距上次调用 {}ms）",
                tool_name,
                self.config.min_interval_ms,
                elapsed.as_millis()
            ));
        }

        // 检查滑动窗口
        if now.duration_since(state.window_start) > Duration::from_secs(self.config.window_secs) {
            state.call_count = 0;
            state.window_start = now;
        }

        if state.call_count >= self.config.max_calls_per_window {
            return Err(format!(
                "工具 '{}' 在 {}秒窗口内已达到最大调用次数 {}",
                tool_name, self.config.window_secs, self.config.max_calls_per_window
            ));
        }

        state.call_count += 1;
        state.last_call = now;
        Ok(())
    }

    /// 扫描输入参数中的敏感信息，返回脱敏后的输入 JSON 字符串
    pub fn sanitize_input(&self, input: &str) -> String {
        if !self.config.scan_sensitive {
            return input.to_string();
        }

        let mut sanitized = input.to_string();

        // 常见的敏感 key 模式
        let sensitive_keys = [
            "api_key",
            "apikey",
            "api_secret",
            "secret",
            "token",
            "password",
            "passwd",
            "auth",
            "credentials",
            "private_key",
            "bearer",
        ];

        // 匹配 "key": "value" 模式，对敏感 key 脱敏 value
        for key in &sensitive_keys {
            // 简单的 key-value 替换
            let patterns = [format!("\"{}\":\"", key), format!("\"{}\": \"", key)];
            for pat in &patterns {
                if let Some(start) = sanitized.find(pat) {
                    let val_start = start + pat.len();
                    if let Some(remaining) = sanitized.get(val_start..)
                        && let Some(end) = remaining.find('"')
                    {
                        let val_len = end;
                        if val_len > 4 {
                            sanitized
                                .replace_range(val_start..val_start + val_len, "***REDACTED***");
                        } else {
                            sanitized.replace_range(val_start..val_start + val_len, "***");
                        }
                    }
                }
            }
        }

        sanitized
    }

    /// 扫描输出内容中的敏感信息，返回是否检测到敏感信息
    pub fn scan_output(&self, output: &str) -> bool {
        if !self.config.scan_sensitive {
            return false;
        }

        // 使用字符串匹配检测常见密钥模式
        let secret_patterns = [
            "sk-",         // OpenAI / Anthropic key prefix
            "sk-ant-",     // Anthropic API key
            "ghp_",        // GitHub personal access token
            "github_pat_", // GitHub fine-grained PAT
            "AKIA",        // AWS access key
            "-----BEGIN RSA PRIVATE KEY-----",
            "-----BEGIN EC PRIVATE KEY-----",
            "-----BEGIN DSA PRIVATE KEY-----",
            "-----BEGIN OPENSSH PRIVATE KEY-----",
            "-----BEGIN PRIVATE KEY-----",
        ];

        for pattern in &secret_patterns {
            if output.contains(pattern) {
                return true;
            }
        }

        // 检测 JWT 令牌模式 (三段 base64 编码，以 eyJ 开头)
        if output.contains("eyJ") {
            let parts: Vec<&str> = output.split('.').collect();
            if parts.len() >= 3 {
                for i in 0..parts.len() - 2 {
                    if parts[i].ends_with("eyJ")
                        || parts[i].contains("eyJ")
                        || (parts[i].len() > 20
                            && parts[i + 1].len() > 20
                            && parts[i + 2].len() > 10)
                    {
                        // 提取 JWT 起始部分（去除前缀文本）
                        let header = if let Some(pos) = parts[i].rfind("eyJ") {
                            &parts[i][pos..]
                        } else {
                            parts[i]
                        };
                        // 简单启发式：三个连续的长段 = 可能的 JWT
                        let is_base64 = |s: &str| {
                            s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                        };
                        if is_base64(header) && is_base64(parts[i + 1]) {
                            return true;
                        }
                    }
                }
            }
        }

        false
    }

    /// 记录审计条目（内存 + 持久化）
    pub async fn log(&self, entry: AuditEntry) {
        // 落库：走实体 API，占位符风格与方言差异由 sea-orm 处理
        if let Some(db) = AUDIT_DB.get() {
            let model = audit_log::ActiveModel {
                timestamp: Set(entry.timestamp),
                tool_name: Set(entry.tool_name.clone()),
                conversation_id: Set(entry.conversation_id.clone()),
                success: Set(entry.success as i32),
                duration_ms: Set(entry.duration_ms as i64),
                output_preview: Set(entry.output_preview.clone()),
                has_sensitive_input: Set(entry.has_sensitive_input as i32),
                has_sensitive_output: Set(entry.has_sensitive_output as i32),
                // `id` 是自增主键，保持 NotSet 交给数据库分配
                ..Default::default()
            };
            if let Err(e) = audit_log::Entity::insert(model).exec(db).await {
                // 审计写失败不应中断工具执行，但也绝不能静默吞掉：原实现用
                // `let _ =` 丢弃全部错误，正是「审计从未落库」长期无人察觉的原因。
                tracing::error!("[ToolAuditor] 审计落库失败 (tool={}): {e}", entry.tool_name);
            }
        }
        // 内存日志
        let mut log = self.log.write().await;
        log.push(entry);
        if log.len() > self.config.max_log_entries {
            log.remove(0);
        }
    }

    /// 获取最近 N 条审计日志
    pub async fn recent_logs(&self, n: usize) -> Vec<AuditEntry> {
        let log = self.log.read().await;
        let start = log.len().saturating_sub(n);
        log[start..].to_vec()
    }

    /// 按工具名筛选审计日志
    pub async fn logs_by_tool(&self, tool_name: &str) -> Vec<AuditEntry> {
        let log = self.log.read().await;
        log.iter().filter(|e| e.tool_name == tool_name).cloned().collect()
    }

    /// 获取审计摘要
    pub async fn summary(&self) -> AuditSummary {
        let log = self.log.read().await;
        let total = log.len();
        let failed = log.iter().filter(|e| !e.success).count();
        let sensitive_inputs = log.iter().filter(|e| e.has_sensitive_input).count();
        let sensitive_outputs = log.iter().filter(|e| e.has_sensitive_output).count();
        let avg_duration = if total > 0 {
            log.iter().map(|e| e.duration_ms).sum::<u64>() / total as u64
        } else {
            0
        };

        let mut tool_counts: HashMap<String, u32> = HashMap::new();
        for entry in log.iter() {
            *tool_counts.entry(entry.tool_name.clone()).or_insert(0) += 1;
        }

        AuditSummary {
            total_calls: total as u64,
            failed_calls: failed as u64,
            sensitive_input_detected: sensitive_inputs as u64,
            sensitive_output_detected: sensitive_outputs as u64,
            avg_duration_ms: avg_duration,
            top_tools: tool_counts.into_iter().collect(),
        }
    }
}

impl Default for ToolAuditor {
    fn default() -> Self {
        Self::new(AuditConfig::default())
    }
}

/// 审计摘要
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditSummary {
    pub total_calls: u64,
    pub failed_calls: u64,
    pub sensitive_input_detected: u64,
    pub sensitive_output_detected: u64,
    pub avg_duration_ms: u64,
    pub top_tools: Vec<(String, u32)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_api_key() {
        let auditor = ToolAuditor::default();
        let input = r#"{"api_key":"sk-abc123secret","query":"test"}"#;
        let sanitized = auditor.sanitize_input(input);
        assert!(!sanitized.contains("sk-abc123secret"));
        assert!(sanitized.contains("***"));
        assert!(sanitized.contains("\"query\":\"test\""));
    }

    #[test]
    fn test_scan_output_jwt() {
        let auditor = ToolAuditor::default();
        let output = "Here is a token: eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";
        assert!(auditor.scan_output(output));
    }

    #[test]
    fn test_no_false_positive() {
        let auditor = ToolAuditor::default();
        let output = "Normal text without any secrets";
        assert!(!auditor.scan_output(output));
    }
}
