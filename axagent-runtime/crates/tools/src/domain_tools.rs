// SPDX-License-Identifier: AGPL-3.0-only

//! 业务域工具槽位（依赖倒置）
//!
//! # 为什么需要
//!
//! `tools` 是通用工具系统（hybrid），本身不含业务域知识。股票域的 54 个 astock
//! MCP 工具此前在这里**直接** `use axagent_astock_data::…` 取定义、取执行实现 ——
//! 域整体迁出为插件时，这条引用会把域 crate 永久钉在工具层里（PLAN §54.3 B5）。
//!
//! 槽位分两段注册，因为两段的可用时机不同：
//!
//! ① **定义** [`register_tool_definitions`]：纯静态 schema，必须在工具注册表
//!    （`UnifiedToolRegistry::new` → `init_all`）构建**之前**注册 —— 注册表按定义
//!    建实例。本段不依赖任何客户端。
//! ② **执行器** [`register_tool_executor`]：需要已构造好的域客户端，在客户端就绪后
//!    注册即可 —— 只在 `Tool::call` 时才查。
//!
//! 未注册定义 ⇒ 该域工具不进注册表（静默，语义是「域未安装」）；
//! 未注册执行器 ⇒ 调用显式报错并点名缺失的接线（不静默）。
//!
//! ⚠ 本模块只认「域标识 + schema + 执行回调」，不认任何具体域 —— 新增业务域
//! 迁出时在域侧实现 [`DomainToolExecutor`] 并注册，本文件不需要改。

use async_trait::async_trait;
use parking_lot::RwLock;
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};

/// 域工具执行器。
///
/// 返回 `String`（工具输出文本）而不是 `Value`：与宿主既有 MCP 执行口
/// （`execute_mcp_tool -> Result<String, String>`）同形，避免多一层 JSON 往返。
#[async_trait]
pub trait DomainToolExecutor: Send + Sync {
    /// 域标识，须与 [`register_tool_definitions`] 注册时用的标识逐字一致。
    fn domain(&self) -> &str;

    /// 执行该域的一个工具。
    async fn execute(&self, tool_name: &str, input: &Value) -> Result<String, String>;
}

/// 已注册定义表：`(域标识, schema 列表)`。
type DomainDefinitions = Vec<(String, Vec<Value>)>;

/// 已注册定义。同域重复注册为替换（幂等）。
static DEFINITIONS: LazyLock<RwLock<DomainDefinitions>> = LazyLock::new(|| RwLock::new(Vec::new()));

/// 已注册执行器（每个域至多一个，同域重复注册为替换）。
static EXECUTORS: LazyLock<RwLock<Vec<Arc<dyn DomainToolExecutor>>>> =
    LazyLock::new(|| RwLock::new(Vec::new()));

/// 定义变更代号。读侧（审计豁免集合）按它记忆化 —— 注册发生在启动早期、
/// 读发生在运行期，用代号失效比「OnceLock 一次定终身」安全：后者一旦在
/// 注册前被读过就永远读不到定义。
static REVISION: AtomicU64 = AtomicU64::new(0);

/// 注册（或替换）某个域的全部工具定义。
///
/// 每个 schema 为 `{"name","description","inputSchema"}`。无名或空名 schema
/// 在实例化时会被跳过（见 `tools::domain_mcp`），此处不校验。
pub fn register_tool_definitions(domain: &str, definitions: Vec<Value>) {
    let mut guard = DEFINITIONS.write();
    guard.retain(|(d, _)| d != domain);
    guard.push((domain.to_string(), definitions));
    REVISION.fetch_add(1, Ordering::SeqCst);
}

/// 定义变更代号（只增不减，进程级）。
pub fn revision() -> u64 {
    REVISION.load(Ordering::SeqCst)
}

/// 全部已注册定义，按「所属域 + schema」展开。
pub fn all_definitions() -> Vec<(String, Value)> {
    DEFINITIONS
        .read()
        .iter()
        .flat_map(|(domain, defs)| defs.iter().cloned().map(|d| (domain.clone(), d)))
        .collect()
}

/// 注册（或替换）某域的工具执行器。
pub fn register_tool_executor(executor: Arc<dyn DomainToolExecutor>) {
    let mut guard = EXECUTORS.write();
    guard.retain(|e| e.domain() != executor.domain());
    guard.push(executor);
}

/// 取某域的执行器。
pub fn find_executor(domain: &str) -> Option<Arc<dyn DomainToolExecutor>> {
    EXECUTORS.read().iter().find(|e| e.domain() == domain).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitions_are_keyed_by_domain_and_replaced_idempotently() {
        let domain = "__fixture_domain_tools_a__";
        register_tool_definitions(domain, vec![serde_json::json!({"name": "a1"})]);
        register_tool_definitions(domain, vec![serde_json::json!({"name": "a2"})]);
        let mine: Vec<Value> =
            all_definitions().into_iter().filter(|(d, _)| d == domain).map(|(_, v)| v).collect();
        assert_eq!(mine.len(), 1, "同域重复注册必须是替换而不是追加");
        assert_eq!(mine[0]["name"], "a2");
    }

    #[test]
    fn revision_advances_on_registration() {
        let before = revision();
        register_tool_definitions("__fixture_domain_tools_b__", Vec::new());
        assert!(revision() > before, "注册必须推进代号，否则读侧缓存永不失效");
    }

    #[test]
    fn executor_lookup_is_by_domain() {
        struct Fixture;
        #[async_trait]
        impl DomainToolExecutor for Fixture {
            fn domain(&self) -> &str {
                "__fixture_domain_tools_c__"
            }
            async fn execute(&self, _name: &str, _input: &Value) -> Result<String, String> {
                Ok("ok".into())
            }
        }
        register_tool_executor(Arc::new(Fixture));
        assert!(find_executor("__fixture_domain_tools_c__").is_some());
        assert!(find_executor("__no_such_domain__").is_none());
    }
}
