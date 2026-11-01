// SPDX-License-Identifier: AGPL-3.0-only

//! 工具调用审计系统
//!
//! 提供：
//! 1. 工具调用频率限制（Rate Limiting）
//! 2. 输入参数敏感信息过滤
//! 3. 输出内容敏感信息扫描
//! 4. 调用审计日志

// `audit_log` 实体定义在 axagent-entities，但 hybrid 层（tools）禁止直接依赖 entities；
// 经 axagent-dao（implementor）整包 re-export 访问（AGENTS.md 分层铁律，与
// `narrative_structure.rs` 走 `axagent_dao::axagent_entities::...` 同款先例）。
use axagent_dao::axagent_entities::audit_log;
use sea_orm::{ConnectionTrait, DatabaseConnection, EntityTrait, Schema, Set};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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

/// 限流违规的类别。
///
/// 分开的用处：调用方（`registry.rs`）据此选择**错误码**。P1-1 修复前二者都被
/// 塞进 `ToolError::permission_denied` ⇒ 模型把「调用太快」读成「没权限」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimitViolationKind {
    /// 距该工具上次调用不足 `AuditConfig::min_interval_ms`
    MinInterval,
    /// 滑动窗口内调用次数已达 `AuditConfig::max_calls_per_window`
    WindowExceeded,
}

/// 一次限流违规（结构化），供调用方构造错误码 + 决定退避。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitViolation {
    pub kind: RateLimitViolationKind,
    /// 说明文本（**不含工具名** —— 工具名由 `ToolError::rate_limited` 统一前缀，
    /// 避免两处各拼一遍而漂移）。
    pub message: String,
    /// 距可重试还需等待的毫秒数。
    pub retry_after_ms: u64,
}

/// 名单里**不豁免**最小间隔的工具（写类 / 有副作用）。
///
/// **评审单点**：两个只读数据源新增写类工具时，在此登记（有守护测试断言本表每个
/// 名字都是**真实存在**的工具名，防「幽灵条目」，并为豁免集合做排除）。
///
/// - `dojo_create_plan` / `dojo_revise_plan` / `dojo_execute_plan`：计划状态机，
///   三者都会落库；
/// - `optimize_attention_weights`：把优化后的权重写回。
const MUTATING_DATA_TOOL_NAMES: &[&str] =
    &["dojo_create_plan", "dojo_revise_plan", "dojo_execute_plan", "optimize_attention_weights"];

/// 只读取数工具的**豁免集合**（P1-2, 2026-09-21）——单点派生，勿手抄。
///
/// ## 为什么需要豁免
///
/// 限流状态按 **`tool_name` 单键**存放（`ToolAuditor::rate_limits`），而股票分析
/// 工作流的多个分析师节点在 `p-analysts` 容器里是**并行**的。两个节点先后调同一个
/// **只读取数**工具时必然互撞 —— 实测原文：
/// `工具 'get_stock_margin_data' 调用过于频繁，最小间隔 200ms（当前距上次调用 48ms）`。
/// 这不是滥用，是并行取数的正常形态。
///
/// ## 判据
///
/// 最小间隔防的是「同一个调用方反复锤同一个工具」。对**只读、幂等、无副作用**的
/// 本地数据源取数工具，多个并行节点共调**不构成滥用** ⇒ 豁免最小间隔。
/// **滑动窗口上限（`max_calls_per_window`）照旧生效** —— 那才是防真滥用的那道闸，
/// 本豁免只摘掉「200ms 单键间隔」这一条。
///
/// ## 名单来源（单点派生）
///
/// 取两个只读数据源的 schema 清单函数 —— 与
/// `seed_consistency_tests::resolvable_tool_names()` 用的是**同一对权威来源**
/// （`tools` crate 已直接依赖这两个 crate，无需手抄、无需改 13 处
/// `UnifiedToolRegistry::new()` 构造点）。手抄 58 个工具名会立刻腐烂。
///
/// ⚠ **不是整包豁免**：清单内混有写类工具，由 [`MUTATING_DATA_TOOL_NAMES`] 逐个剔除
/// （实测 58 个工具里有 4 个写类/有副作用）。
fn read_only_data_tool_names() -> &'static HashSet<String> {
    static NAMES: OnceLock<HashSet<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let mut set = HashSet::new();
        for schema in axagent_astock_data::mcp_tools::stock_mcp_tools()
            .into_iter()
            .chain(axagent_analysis_engine::mcp_tools::industry_chain_mcp_tools())
        {
            if let Some(name) = schema.get("name").and_then(|v| v.as_str()) {
                set.insert(name.to_string());
            }
        }
        for mutating in MUTATING_DATA_TOOL_NAMES {
            set.remove(*mutating);
        }
        set
    })
}

/// 该工具是否豁免最小间隔。
///
/// 除裸名外还认 `{server_id}/{tool_name}` 形态 —— MCP 工具在 `mcp_tools` 里就是按
/// 这个 key 存的。**不处理这层会让豁免对某种调用形态静默失效**（与本轮修的
/// `filter_map` 静默丢弃同族），故显式覆盖。
fn is_min_interval_exempt(tool_name: &str) -> bool {
    let names = read_only_data_tool_names();
    if names.contains(tool_name) {
        return true;
    }
    tool_name.split_once('/').is_some_and(|(_, bare)| names.contains(bare))
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

    /// 检查频率限制，返回 `Ok(())` 表示允许，`Err(RateLimitViolation)` 表示触发限制。
    ///
    /// 两类约束的**处理不同**（P1-2, 2026-09-21）：
    /// - **最小间隔**：只读取数工具豁免（[`read_only_data_tool_names`]），
    ///   因为并行分析师节点共调同一只读取数工具是正常形态，不是滥用；
    /// - **滑动窗口上限**：对**所有**工具生效，它是防真滥用的那道闸。
    ///
    /// 返回结构化违规而非 `String`：调用方据此选**错误码**
    /// （`ToolError::rate_limited`），不再复用 `permission_denied`（P1-1）。
    pub async fn check_rate_limit(&self, tool_name: &str) -> Result<(), RateLimitViolation> {
        let mut limits = self.rate_limits.write().await;
        let now = Instant::now();
        let state = limits.entry(tool_name.to_string()).or_insert(RateLimitState {
            last_call: now - Duration::from_secs(60),
            call_count: 0,
            window_start: now,
        });

        // 检查最小间隔（只读取数工具豁免）
        let elapsed = now.duration_since(state.last_call);
        if !is_min_interval_exempt(tool_name)
            && elapsed < Duration::from_millis(self.config.min_interval_ms)
        {
            return Err(RateLimitViolation {
                kind: RateLimitViolationKind::MinInterval,
                message: format!(
                    "调用过于频繁，最小间隔 {}ms（当前距上次调用 {}ms）；这是\
                     **时序约束、非权限问题**，稍后重试即可",
                    self.config.min_interval_ms,
                    elapsed.as_millis()
                ),
                retry_after_ms: self
                    .config
                    .min_interval_ms
                    .saturating_sub(elapsed.as_millis() as u64),
            });
        }

        // 检查滑动窗口
        if now.duration_since(state.window_start) > Duration::from_secs(self.config.window_secs) {
            state.call_count = 0;
            state.window_start = now;
        }

        if state.call_count >= self.config.max_calls_per_window {
            return Err(RateLimitViolation {
                kind: RateLimitViolationKind::WindowExceeded,
                message: format!(
                    "在 {} 秒窗口内已达到最大调用次数 {}；这是**时序约束、非权限问题**，\
                     稍后重试即可",
                    self.config.window_secs, self.config.max_calls_per_window
                ),
                retry_after_ms: Duration::from_secs(self.config.window_secs)
                    .saturating_sub(now.duration_since(state.window_start))
                    .as_millis() as u64,
            });
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

    // ─────────────────────────────────────────────────────────────────────────
    // P1-2（2026-09-21）：只读取数工具豁免最小间隔
    // ─────────────────────────────────────────────────────────────────────────

    /// 正控：豁免集合必须**非空**，且必须**覆盖本轮真实撞限流的那只工具**。
    ///
    /// 本断言的价值在于把门禁**锚在真实事件**上：若哪天 `stock_mcp_tools()` 改名、
    /// 或派生逻辑写错（例如漏掉 astock-data 那一支），本测试立刻红 ——
    /// 而不是让豁免集合静默变空、P1-2 悄悄退回修复前状态。
    #[test]
    fn read_only_exempt_set_covers_the_incident_tool() {
        let names = read_only_data_tool_names();
        assert!(
            !names.is_empty(),
            "豁免集合为空 ⇒ 派生逻辑失效（`stock_mcp_tools()` / \
             `industry_chain_mcp_tools()` 的 `name` 字段抽取出问题？）"
        );
        // 本轮实测撞限流的那只（原文：`工具 'get_stock_margin_data' 调用过于频繁，
        // 最小间隔 200ms（当前距上次调用 48ms）`），以及本轮新接线的质押工具。
        for must_have in ["get_stock_margin_data", "get_stock_pledge_data", "get_stock_kline"] {
            assert!(
                names.contains(must_have),
                "豁免集合里没有 `{must_have}` ⇒ P1-2 对真实碰撞场景**不生效**（假修复）。\
                 当前集合 {n} 项：{names:?}",
                n = names.len()
            );
        }
        // 规模自证：两个数据源合计声明 60 项，剔除 4 个写类后应远多于 10。
        // 下界只用来抓「只抽到一支数据源」这类静默退化，不追求精确值。
        assert!(
            names.len() >= 50,
            "豁免集合只有 {} 项 —— 预期 ≥ 50（astock-data 58 + 产业链 2 − 写类 4）。\
             先查是不是只抽到了其中一支数据源。",
            names.len()
        );
    }

    /// 豁免**不是整包**：写类/有副作用的工具必须仍受最小间隔约束。
    ///
    /// 同时反查「幽灵条目」：`MUTATING_DATA_TOOL_NAMES` 里每个名字都必须是
    /// 数据源真声明过的工具名 —— 否则剔除动作打在了空气上（判据「幽灵码」同族）。
    #[test]
    fn mutating_tools_are_excluded_and_all_names_are_real() {
        let names = read_only_data_tool_names();
        // 先造全量声明集（不剔除），用来验「剔除项确实存在于声明中」。
        let declared: HashSet<String> = axagent_astock_data::mcp_tools::stock_mcp_tools()
            .into_iter()
            .chain(axagent_analysis_engine::mcp_tools::industry_chain_mcp_tools())
            .filter_map(|t| t.get("name").and_then(|v| v.as_str()).map(String::from))
            .collect();

        for m in MUTATING_DATA_TOOL_NAMES {
            assert!(
                declared.contains(*m),
                "`{m}` 被登记进 MUTATING_DATA_TOOL_NAMES，但它**不是**任何数据源声明的工具名 \
                 ⇒ 幽灵条目（剔除打在空气上）。删掉它，或订正拼写。"
            );
            assert!(
                !names.contains(*m),
                "写类工具 `{m}` 出现在豁免集合里 ⇒ 整包豁免的 bug，它必须受最小间隔约束。"
            );
        }
    }

    /// 行为证明：同名的两次紧邻调用 —— 只读工具放行，非只读工具被拦。
    #[tokio::test]
    async fn min_interval_exempts_read_only_but_still_guards_others() {
        let auditor = ToolAuditor::default(); // min_interval_ms = 200

        // 只读：连续两次紧邻调用都应放行（豁免最小间隔）。
        assert!(
            auditor.check_rate_limit("get_stock_margin_data").await.is_ok(),
            "只读取数工具第一次调用不该被拦"
        );
        assert!(
            auditor.check_rate_limit("get_stock_margin_data").await.is_ok(),
            "只读取数工具**紧邻第二次**调用必须放行 —— 这正是并行分析师节点互撞的场景（P1-2）"
        );

        // 非只读：紧邻第二次必须被拦，且类别是 MinInterval（不是 Window）。
        assert!(auditor.check_rate_limit("some_non_exempt_tool").await.is_ok());
        let violation = auditor
            .check_rate_limit("some_non_exempt_tool")
            .await
            .expect_err("非只读工具的紧邻第二次调用必须被最小间隔拦住");
        assert_eq!(violation.kind, RateLimitViolationKind::MinInterval);
        assert!(violation.retry_after_ms > 0, "应给出可操作的退避时长");
        // 措辞锁：**不得**出现「归因性」表述（那正是模型写成「工具调用被拒绝」的燃料）。
        // 注意「非权限问题」这类**显式否认**不算违规 —— 只有把限流归因成权限/授权缺失才违规。
        for banned in ["权限被拒绝", "权限不足", "未授权", "无权限", "被拒绝"] {
            assert!(
                !violation.message.contains(banned),
                "违规说明含归因性表述「{banned}」⇒ 会被模型读成能力缺失（P1-1 的成因）。\
                 实际文本：{}",
                violation.message
            );
        }
    }

    /// 豁免**只摘掉最小间隔这一条**：滑动窗口上限对只读工具**照旧**生效。
    ///
    /// 这条防的是「为了修 P1-2 把整道限流闸拆了」。
    #[tokio::test]
    async fn window_limit_still_applies_to_exempt_tools() {
        let auditor = ToolAuditor::default(); // max_calls_per_window = 30

        for i in 0..30 {
            assert!(
                auditor.check_rate_limit("get_stock_kline").await.is_ok(),
                "第 {} 次调用不该被拦（窗口上限是 30）",
                i + 1
            );
        }
        let violation = auditor
            .check_rate_limit("get_stock_kline")
            .await
            .expect_err("第 31 次调用必须被滑动窗口上限拦住 —— 豁免不得连带摘掉这道闸");
        assert_eq!(violation.kind, RateLimitViolationKind::WindowExceeded);
    }

    /// `{server_id}/{tool_name}` 形态也要能命中豁免。
    ///
    /// MCP 工具在 `mcp_tools` 里按这个 key 存；不覆盖该形态会让豁免对某种调用方式
    /// **静默失效**（与 `filter_map` 静默丢弃同族），故显式断言。
    #[test]
    fn exempt_lookup_handles_server_qualified_names() {
        assert!(is_min_interval_exempt("get_stock_margin_data"));
        assert!(is_min_interval_exempt("stock/get_stock_margin_data"));
        assert!(!is_min_interval_exempt("stock/dojo_create_plan"));
        assert!(!is_min_interval_exempt("dojo_create_plan"));
    }
}
