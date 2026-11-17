// SPDX-License-Identifier: AGPL-3.0-only

//! 节点执行器工厂注册表（P6.0-① 阶段4）。
//!
//! 目标：新增一个节点类型 = 实现 executor + 注册一条工厂，**不改主壳**。
//! `NodeDispatcher::init_builtin` 与 `WorkEngine::init_dispatcher` 均改为遍历本表，
//! 不再手写 27 条 `register(...)` 与 5 条带依赖构造。
//!
//! 两类工厂：
//! - `Simple`：无依赖，如 `TriggerExecutor::new()`，可在无 ctx 的轻量路径注册；
//! - `Context`：需 master_key / provider_registry / 共享缓存，由 `WorkEngine::init_dispatcher`
//!   构造 ctx 后统一装配（AgentExecutor 特例：返回 ctx 持有的共享 Arc）。
//!
//! 与 `NodeDescriptorRegistry` 的分工：descriptor 管「类型元数据」（kind/config/schema/
//! output_var），本表管「如何构造执行器」。两者按 type_name 字符串对齐；集合不完全相同
//! （descriptor 有 workflowRef 无 executor，executor 有 fallback 无 descriptor），故以
//! 「实际要注册的执行器集合」为准，不强行并表。

use std::collections::HashMap;
use std::sync::Arc;

use axagent_harness::registry::ProviderRegistry;

use super::executors::{AgentExecutor, ProfileCache, ProviderCache};
use super::node_executor_trait::NodeExecutorTrait;

/// 带依赖 executor 的构造上下文（sync 构造期快照，不跨 await）。
pub struct ExecutorFactoryCtx {
    pub master_key: [u8; 32],
    pub provider_registry: Arc<dyn ProviderRegistry>,
    /// AgentExecutor 共享 Arc：工厂返回它本身，保住运行期 setter 热更新语义。
    pub agent_executor: Arc<AgentExecutor>,
    pub agent_provider_cache: Arc<tokio::sync::Mutex<ProviderCache>>,
    pub agent_profile_cache: Arc<tokio::sync::Mutex<ProfileCache>>,
}

/// 执行器工厂：按 ctx 构造一个执行器实例。
pub enum ExecutorFactory {
    /// 无依赖构造（轻量路径 / 测试可用）。
    Simple(fn() -> Arc<dyn NodeExecutorTrait>),
    /// 需上下文构造（由 WorkEngine::init_dispatcher 装配）。
    Context(fn(&ExecutorFactoryCtx) -> Arc<dyn NodeExecutorTrait>),
}

impl ExecutorFactory {
    fn build(&self, ctx: Option<&ExecutorFactoryCtx>) -> Option<Arc<dyn NodeExecutorTrait>> {
        match (self, ctx) {
            (Self::Simple(f), _) => Some(f()),
            (Self::Context(f), Some(ctx)) => Some(f(ctx)),
            (Self::Context(_), None) => None,
        }
    }
}

/// 无依赖工厂条目：type 串 → 构造函数。
type SimpleFactoryEntry = (&'static str, fn() -> Arc<dyn NodeExecutorTrait>);

/// 全局工厂注册表（启动期写、运行期只读；parking_lot 同步锁不跨 await）。
static FACTORIES: std::sync::OnceLock<parking_lot::RwLock<HashMap<&'static str, ExecutorFactory>>> =
    std::sync::OnceLock::new();

fn factories() -> &'static parking_lot::RwLock<HashMap<&'static str, ExecutorFactory>> {
    FACTORIES.get_or_init(|| parking_lot::RwLock::new(HashMap::new()))
}

/// 注册一个执行器工厂。返回 `false` 表示同名已注册（保留先注册者）。
pub fn register_factory(node_type: &'static str, factory: ExecutorFactory) -> bool {
    let mut map = factories().write();
    if map.contains_key(node_type) {
        return false;
    }
    map.insert(node_type, factory);
    true
}

/// 工厂表中的类型名集合（供测试断言集合一致）。
pub fn factory_type_names() -> Vec<&'static str> {
    factories().read().keys().copied().collect()
}

/// 注册所有内置执行器工厂（27 Simple + 5 Context）。
///
/// 由 `WorkEngine::new` 首次调用（在构造 ctx 之前即可，Simple/Context 的注册
/// 都只是登记 fn pointer）。重复调用幂等。
pub fn register_builtin_factories() {
    use super::executors::*;

    // ---- 无依赖 27 个（原 NodeDispatcher::init_builtin 全表）----
    // NodeExecutorTrait::node_type 是实例方法，工厂表 key 需要静态串；
    // 这里用显式字符串表，与实现内的 node_type() 返回值逐字对应（测试锁定）。
    let simple_factories: &[SimpleFactoryEntry] = &[
        ("trigger", || Arc::new(TriggerExecutor::new())),
        ("parallel", || Arc::new(ParallelExecutor::new())),
        ("loop", || Arc::new(LoopExecutor::new())),
        ("merge", || Arc::new(MergeExecutor::new())),
        ("delay", || Arc::new(DelayExecutor::new())),
        ("subWorkflow", || Arc::new(SubWorkflowExecutor::new())),
        ("documentParser", || Arc::new(DocumentParserExecutor::new())),
        ("vectorRetrieve", || Arc::new(VectorRetrieveExecutor::new())),
        ("end", || Arc::new(EndExecutor::new())),
        ("validation", || Arc::new(ValidationExecutor::new())),
        ("tool", || Arc::new(ToolExecutor::new())),
        ("code", || Arc::new(CodeExecutor::new())),
        ("debate", || Arc::new(DebateExecutor::new())),
        ("fallback", || Arc::new(FallbackExecutor::new())),
        ("httpRequest", || Arc::new(HttpRequestExecutor::new())),
        ("databaseQuery", || Arc::new(DatabaseQueryExecutor::new())),
        ("notification", || Arc::new(NotificationExecutor::new())),
        ("approval", || Arc::new(ApprovalExecutor::new())),
        ("fileOperation", || Arc::new(FileOperationExecutor::new())),
        ("dataTransformer", || Arc::new(DataTransformerExecutor::new())),
        ("webhookSend", || Arc::new(WebhookSendExecutor::new())),
        ("logging", || Arc::new(LoggingExecutor::new())),
        ("aggregator", || Arc::new(AggregatorExecutor::new())),
        ("email", || Arc::new(EmailExecutor::new())),
        ("swarm", || Arc::new(SwarmExecutor::new())),
        ("multiAgent", || Arc::new(MultiAgentExecutor::new())),
        ("storage", || Arc::new(StorageExecutor::new())),
    ];
    for (key, f) in simple_factories {
        register_factory(key, ExecutorFactory::Simple(*f));
    }

    // ---- 带依赖 5 个 ----
    use axagent_harness::HasProviderRegistry;

    register_factory(
        "agent",
        ExecutorFactory::Context(|ctx| ctx.agent_executor.clone() as Arc<dyn NodeExecutorTrait>),
    );
    register_factory(
        "llm",
        ExecutorFactory::Context(|ctx| {
            let mut e = LlmExecutor::new(ctx.master_key);
            e.set_provider_registry(ctx.provider_registry.clone());
            Arc::new(e)
        }),
    );
    register_factory(
        "condition",
        ExecutorFactory::Context(|ctx| {
            let mut e = ConditionExecutor::new(ctx.master_key);
            e.set_provider_registry(ctx.provider_registry.clone());
            Arc::new(e)
        }),
    );
    register_factory(
        "llmClassifier",
        ExecutorFactory::Context(|ctx| {
            let mut e = LlmClassifierExecutor::new(ctx.master_key);
            e.set_provider_registry(ctx.provider_registry.clone());
            Arc::new(e)
        }),
    );
    register_factory(
        "switch",
        ExecutorFactory::Context(|ctx| {
            let mut e = SwitchExecutor::new(ctx.master_key);
            e.set_provider_registry(ctx.provider_registry.clone());
            Arc::new(e)
        }),
    );
}

/// 遍历工厂表构造所有可执行执行器。
///
/// - `ctx = None`：只返回 Simple 工厂产物（供 `init_builtin` 轻量路径）；
/// - `ctx = Some`：返回全部 32 个（供 `WorkEngine::init_dispatcher`）。
pub fn build_all(
    ctx: Option<&ExecutorFactoryCtx>,
) -> Vec<(&'static str, Arc<dyn NodeExecutorTrait>)> {
    register_builtin_factories();
    let map = factories().read();
    map.iter().filter_map(|(key, f)| f.build(ctx).map(|exec| (*key, exec))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 工厂表 key 集合必须覆盖 32 种实际执行器（含 fallback、不含 workflowRef）。
    /// 用 >= 32：外部测试可能已通过 `register_factory` 追加类型，这是设计允许的扩展。
    #[test]
    fn factory_keys_cover_all_executors() {
        register_builtin_factories();
        let names = factory_type_names();
        assert!(names.len() >= 32, "工厂应至少覆盖 32 个内置执行器");
        for key in ["fallback", "workflowRef", "agent", "llm", "storage"] {
            if key == "workflowRef" {
                assert!(!names.contains(&key), "workflowRef 无执行器，不应有工厂");
            } else {
                assert!(names.contains(&key), "缺少工厂: {key}");
            }
        }
    }

    /// 阶段4 验收判据1（Rust 侧）：外部节点仅靠实现 executor + 注册一条工厂
    /// 即出现在 `build_all` 结果中，`NodeDispatcher::init_builtin` 零改。
    /// 注意：端到端 DAG 执行仍受 enum 共存限制，留到阶段6 删除 enum 后验收。
    #[test]
    fn external_factory_appears_in_build_all_without_touching_dispatcher() {
        struct TestOnlyExecutor;
        #[async_trait::async_trait]
        impl NodeExecutorTrait for TestOnlyExecutor {
            fn node_type(&self) -> &'static str {
                "testOnlyExternalExecutor"
            }
            async fn execute(
                &self,
                _node: &axagent_harness::workflow_types::WorkflowNode,
                _context: &super::super::execution_state::ExecutionState,
            ) -> Result<
                super::super::node_executor_trait::NodeOutput,
                super::super::node_executor_trait::NodeError,
            > {
                Ok(super::super::node_executor_trait::NodeOutput {
                    output: serde_json::json!({"ok": true}),
                    output_var: None,
                    control: None,
                })
            }
        }

        assert!(register_factory(
            "testOnlyExternalExecutor",
            ExecutorFactory::Simple(|| Arc::new(TestOnlyExecutor)),
        ));
        let built = build_all(None);
        assert!(
            built.iter().any(|(key, exec)| *key == "testOnlyExternalExecutor"
                && exec.node_type() == "testOnlyExternalExecutor"),
            "外部工厂应出现在 build_all 结果中"
        );
        assert!(
            !register_factory(
                "testOnlyExternalExecutor",
                ExecutorFactory::Simple(|| Arc::new(TestOnlyExecutor)),
            ),
            "重复注册同名工厂应被拒绝"
        );
    }
}
