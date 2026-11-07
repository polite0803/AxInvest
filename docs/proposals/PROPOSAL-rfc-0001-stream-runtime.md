> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# RFC-0001: Workflow Stream Runtime（WorSR）

**Status**: DRAFT
**Author**: [AxAgent 团队]
**Created**: 2026-06-06
**Supersedes**: —
**References**: [stream-architecture.md](../designs/stream-architecture.md)

---

## 1. Summary

引入**流式执行运行时（Workflow Stream Runtime, WorSR）**，让现有 28 节点工作流引擎获得 4 个新能力原语：**Stream** / **Subscription** / **Producer-Consumer** / **Observation**。不替换现有 DAG 引擎，仅通过双写 + 元数据扩展**叠加**新能力。落地 4 个 sprint，渐进验证能力原语能服务任何事件驱动 / 决策驱动 / 数据流驱动的业务场景。

---

## 2. Motivation

参见 [stream-architecture.md §Problem Statement](../designs/stream-architecture.md)。简述：

- 现有 28 节点 DAG 引擎是"流程图 + 节点变量传递"模型
- 节点输出是**末值**（`HashMap<String, Value>`），不支持查询、订阅、回放
- 跨工作流协同必须经过 Trigger 节点，缺少声明式订阅
- 可观察偏研发视角，缺业务无关的"流聚合"原语

**WorSR 的目标**：让节点输出天然成为可寻址、可订阅、可重放的事件流，**不绑业务**。

---

## 3. Detailed Design

### 3.1 新增 Crate：`runtime-core/stream_types`

**位置**：`src-tauri/crates/runtime-core/stream_types/`

**Cargo.toml 关键依赖**：
```toml
[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["sync"] }
thiserror = "1"
axagent_core = { path = "../core" }  # 复用 JsonSchema
```

**模块结构**：
```
stream_types/src/
├── lib.rs              # 重新导出
├── stream.rs           # Stream trait
├── subscription.rs     # Subscription 类型
├── window.rs           # Window / Filter / Aggregate
├── schema.rs           # JsonSchema 校验（薄封装）
├── event.rs            # WorkflowStreamEvent
└── error.rs            # StreamError 错误类型
```

#### 3.1.1 `stream.rs` — Stream Trait

```rust
use std::sync::Arc;
use tokio::sync::mpsc;
use serde::{Serialize, Deserialize};
use crate::{Window, Filter, WorkflowStreamEvent, StreamError};

/// 流：有类型时间序列
/// 强类型 + 动态 schema 校验
pub trait Stream<T>: Send + Sync
where
    T: Serialize + for<'de> Deserialize<'de> + 'static,
{
    /// 订阅流事件（推送模式）
    /// 返回一个 SubscriptionHandle，可被取消
    /// 如果被订阅方未启动，订阅进入 Pending 状态
    fn subscribe(
        &self,
        filter: Option<Filter>,
    ) -> Result<SubscriptionHandle, StreamError>;

    /// 聚合查询（拉取模式）
    fn query(
        &self,
        window: Window,
    ) -> Result<Vec<WorkflowStreamEvent<T>>, StreamError>;

    /// 实时尾追（流式拉取）
    fn tail(&self) -> mpsc::Receiver<WorkflowStreamEvent<T>>;
}

/// 流构造器（创建流的工厂）
pub trait StreamFactory: Send + Sync {
    fn create<T>(&self, topic: &str) -> Arc<dyn Stream<T>>;
}
```

#### 3.1.2 `subscription.rs`

```rust
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use crate::{Filter, StreamError};

/// 订阅句柄（不透明、强引用计数）
#[derive(Clone)]
pub struct SubscriptionHandle {
    inner: Arc<SubscriptionInner>,
}

struct SubscriptionInner {
    id: u64,
    topic: String,
    cancel_token: CancellationToken,
}

impl SubscriptionHandle {
    /// 显式取消订阅
    pub async fn cancel(self) -> Result<(), StreamError>;

    /// 检查订阅是否仍然活跃
    pub fn is_active(&self) -> bool;
}

/// 订阅构建器（强制要求绑定取消令牌）
pub struct SubscriptionBuilder {
    topic: String,
    filter: Option<Filter>,
    cancel_token: Option<CancellationToken>,
}

impl SubscriptionBuilder {
    pub fn new(topic: impl Into<String>) -> Self;
    pub fn filter(mut self, f: Filter) -> Self;
    pub fn with_cancel_token(mut self, token: CancellationToken) -> Self;
    pub async fn build(self) -> Result<SubscriptionHandle, StreamError>;
}
```

#### 3.1.3 `window.rs`

```rust
use serde::{Serialize, Deserialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Window {
    /// 最近 N 条
    Last(usize),
    /// 最近一段时间
    Since(Duration),
    /// 时间范围（start..end）
    Range { start: i64, end: i64 },
    /// 全部
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Filter {
    /// payload 字段匹配
    FieldEquals { name: String, value: serde_json::Value },
    /// payload 类型匹配
    TypeMatches(String),
    /// 自定义谓词（字符串表达式，参考 Rhai 集成）
    Predicate(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Aggregate {
    Count,
    Sum(String),     // field name
    Avg(String),
    Min(String),
    Max(String),
}
```

#### 3.1.4 `event.rs`

```rust
use serde::{Serialize, Deserialize};
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStreamEvent<T> {
    pub id: u64,
    pub topic: String,
    pub timestamp: SystemTime,
    pub payload: T,
    /// 来源节点（如果适用）
    pub source_node: Option<String>,
    /// 来源工作流（如果适用）
    pub source_workflow: Option<String>,
}
```

#### 3.1.5 `error.rs`

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum StreamError {
    #[error("subscription canceled")]
    Canceled,

    #[error("topic not found: {0}")]
    TopicNotFound(String),

    #[error("schema validation failed: {0}")]
    SchemaValidation(String),

    #[error("query timeout after {0:?}")]
    QueryTimeout(std::time::Duration),

    #[error("cyclic subscription detected: {0}")]
    CyclicSubscription(String),

    #[error("internal: {0}")]
    Internal(String),
}
```

---

### 3.2 新增模块：`runtime/stream_runtime`

**位置**：`src-tauri/crates/runtime/src/stream_runtime/`

**模块结构**：
```
stream_runtime/
├── mod.rs              # 重新导出
├── in_process.rs       # InProcessBus 默认实现
└── registry.rs         # 全局流注册表
```

#### 3.2.1 `in_process.rs` — InProcessBus

```rust
use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::{RwLock, mpsc};
use tokio_util::sync::CancellationToken;
use async_trait::async_trait;
use runtime_core_stream_types::*;

#[async_trait]
pub trait InProcessBus: Send + Sync {
    /// 创建一个 topic 的流
    async fn create_topic<T>(&self, topic: &str) -> Arc<dyn Stream<T>>
    where
        T: Serialize + for<'de> Deserialize<'de> + 'static;

    /// 注入事件到 topic
    async fn inject<T>(&self, topic: &str, event: WorkflowStreamEvent<T>) -> Result<(), StreamError>
    where
        T: Serialize + for<'de> Deserialize<'de> + 'static;

    /// 检查 topic 是否存在
    async fn topic_exists(&self, topic: &str) -> bool;
}

pub struct DefaultInProcessBus {
    topics: RwLock<HashMap<String, Box<dyn AnyStream>>>,
}

#[async_trait]
impl InProcessBus for DefaultInProcessBus {
    async fn create_topic<T>(&self, topic: &str) -> Arc<dyn Stream<T>>
    where
        T: Serialize + for<'de> Deserialize<'de> + 'static,
    {
        // 实现：使用 tokio::broadcast 内部多生产者单消费者
    }

    async fn inject<T>(&self, topic: &str, event: WorkflowStreamEvent<T>) -> Result<(), StreamError> {
        // 写入 topic 内部的 broadcast::Sender
    }
}
```

#### 3.2.2 `registry.rs` — 全局流注册表

```rust
use std::sync::Arc;
use std::any::Any;
use tokio::sync::RwLock;
use std::collections::HashMap;

/// 全局流注册表（单例）
pub struct StreamRegistry {
    inner: Arc<RwLock<HashMap<String, Box<dyn AnyStream>>>>,
}

pub trait AnyStream: Send + Sync {
    fn as_any(&self) -> &dyn Any;
}

impl StreamRegistry {
    pub fn global() -> &'static Self;  // 全局单例
    pub async fn register(&self, topic: &str, stream: Box<dyn AnyStream>);
    pub async fn get(&self, topic: &str) -> Option<Box<dyn AnyStream>>;
    pub async fn list(&self) -> Vec<String>;
}
```

**约束**：注册表在 lib.rs 启动时初始化，关闭时不主动清理（依赖进程退出）。

---

### 3.3 新增模块：`rt-workflow/stream_engine.rs`

**位置**：`src-tauri/crates/rt-workflow/src/stream_engine.rs`

#### 3.3.1 双写适配器（核心）

```rust
use runtime_core_stream_types::*;
use rt_workflow::workflow_execution::WorkflowExecution;
use std::sync::Arc;
use tokio::sync::RwLock;
use axagent_core::schema::JsonSchema;

pub struct StreamLogger {
    bus: Arc<dyn InProcessBus>,
    schema: Arc<JsonSchema>,
}

impl StreamLogger {
    /// 节点执行完成时调用
    /// 同时写 results（旧视图，兼容）和 stream_log（新视图，流）
    pub async fn log_node_output<T>(
        &self,
        execution: &mut WorkflowExecution,
        node_id: &str,
        topic: &str,
        payload: T,
    ) -> Result<(), StreamError>
    where
        T: Serialize + for<'de> Deserialize<'de> + 'static,
    {
        // 1. 写 results（兼容旧视图，best-effort 不阻塞）
        if let Ok(json) = serde_json::to_value(&payload) {
            execution.results.insert(node_id.to_string(), json);
        }

        // 2. 写 stream_log（新视图）
        let event = WorkflowStreamEvent {
            id: execution.next_stream_id(),
            topic: topic.to_string(),
            timestamp: std::time::SystemTime::now(),
            payload,
            source_node: Some(node_id.to_string()),
            source_workflow: Some(execution.workflow_id.clone()),
        };
        self.bus.inject(topic, event).await?;

        Ok(())
    }
}
```

#### 3.3.2 WorkflowExecution 扩展

```rust
// 在现有 WorkflowExecution struct 上扩展
pub struct WorkflowExecution {
    // ... 现有字段 ...

    /// 新增：流日志（BTreeMap 按 node_id 索引）
    pub stream_log: BTreeMap<String, Vec<WorkflowStreamEvent<serde_json::Value>>>,

    /// 新增：流 ID 计数器
    next_stream_id: AtomicU64,
}

impl WorkflowExecution {
    pub fn next_stream_id(&self) -> u64 {
        self.next_stream_id.fetch_add(1, Ordering::SeqCst)
    }
}
```

#### 3.3.3 锁顺序纪律

新增 `stream_subs: Arc<RwLock<HashMap<SubscriptionId, SubscriptionHandle>>>` 时遵守锁顺序：

```
workflows → executions → node_breakers → cancel_tokens → stream_subs
```

**绝对禁止**违反此顺序。code review 时强制检查。

#### 3.3.4 跨工作流订阅静态检测

工作流启动前检测订阅图是否有环：

```rust
pub fn detect_cyclic_subscriptions(
    subscriptions: &[WorkflowSubscription],
) -> Result<(), StreamError> {
    // DFS 检测有向环
    let mut graph = HashMap::new();
    for sub in subscriptions {
        graph.entry(sub.from.clone()).or_insert_with(Vec::new).push(sub.to.clone());
    }
    // ... 简化 DFS ...
    Ok(())
}
```

---

### 3.4 节点元数据扩展

**位置**：`src/components/workflow/Nodes/BaseNode.tsx`

```typescript
// BaseNode.tsx
import { StreamDecl, JsonSchema } from '@/types/workflow-stream';

export abstract class BaseNode {
  // ... 现有字段 ...

  /**
   * 节点对外暴露的流（默认空数组，子类按需覆盖）
   * Sprint 1 仅在 BaseNode 提供默认值，不要求 28 节点逐个显式声明
   */
  produces: StreamDecl[] = [];

  /**
   * 节点订阅的流（默认空数组）
   */
  consumes: StreamDecl[] = [];

  /**
   * 由 node_type 自动推导的节点角色（不可手动设置）
   * 决策点：node_type 包含 'classifier' 或 'condition' 或 'switch' 或 'approval'
   * 副作用：node_type 包含 'http' / 'send' / 'db_' / 'email' / 'webhook'
   */
  get roles(): NodeRole[] {
    return deriveRolesFromNodeType(this.node_type);
  }
}

function deriveRolesFromNodeType(nodeType: string): NodeRole[] {
  const roles: NodeRole[] = ['producer', 'consumer'];
  if (/classifier|condition|switch|approval/.test(nodeType)) {
    roles.push('decision_point');
  }
  if (/^http_|_send$|^db_|^email|^webhook/.test(nodeType)) {
    roles.push('effect_node');
  }
  return roles;
}

export type NodeRole = 'producer' | 'consumer' | 'decision_point' | 'effect_node';
```

**元数据改造总行数 ≤ 100 行**（仅 BaseNode 增加方法 + 类型定义）。

---

### 3.5 前端类型定义

**位置**：`src/types/workflow-stream.ts`

```typescript
import { JSONSchemaType } from 'ajv';

export interface StreamDecl {
  name: string;
  schema: JSONSchemaType<any>;
}

export type NodeRole = 'producer' | 'consumer' | 'decision_point' | 'effect_node';

export interface WorkflowStreamEvent<T = unknown> {
  id: number;
  topic: string;
  timestamp: number;  // unix millis
  payload: T;
  sourceNode?: string;
  sourceWorkflow?: string;
}

export interface SubscriptionHandle {
  id: number;
  topic: string;
  cancel: () => Promise<void>;
  isActive: () => boolean;
}

export interface WorkflowSubscription {
  source: string;  // topic or node_id
  filter?: import('./workflow-stream').StreamFilter;
}
```

**类型生成**：使用 `ts-rs`（Rust 端 `stream_types` crate 单一源 → TS 类型自动生成）。**禁止手写重复类型**。

---

### 3.6 Store 扩展

**位置**：`src/stores/domain/workflowStreamStore.ts`

```typescript
import { create } from 'zustand';
import type { WorkflowStreamEvent, SubscriptionHandle } from '@/types/workflow-stream';

interface WorkflowStreamState {
  events: Map<string, WorkflowStreamEvent[]>;  // topic -> events
  subscriptions: Map<number, SubscriptionHandle>;  // handle_id -> handle
  error: string | null;

  // actions
  appendEvent: (topic: string, event: WorkflowStreamEvent) => void;
  registerSubscription: (handle: SubscriptionHandle) => void;
  unregisterSubscription: (handleId: number) => void;
  clearTopic: (topic: string) => void;
  setError: (error: string | null) => void;
}

export const useWorkflowStreamStore = create<WorkflowStreamState>((set, get) => ({
  events: new Map(),
  subscriptions: new Map(),
  error: null,

  appendEvent: (topic, event) => set((state) => {
    const existing = state.events.get(topic) || [];
    const updated = new Map(state.events);
    updated.set(topic, [...existing, event].slice(-1000));  // 内存 ring buffer
    return { events: updated };
  }),

  registerSubscription: (handle) => set((state) => {
    const updated = new Map(state.subscriptions);
    updated.set(handle.id, handle);
    return { subscriptions: updated };
  }),

  unregisterSubscription: (handleId) => set((state) => {
    const updated = new Map(state.subscriptions);
    updated.delete(handleId);
    return { subscriptions: updated };
  }),

  clearTopic: (topic) => set((state) => {
    const updated = new Map(state.events);
    updated.delete(topic);
    return { events: updated };
  }),

  setError: (error) => set({ error }),
}));
```

**与现有 `streamStore.ts` 区分**：
- `streamStore.ts` → ChatStream（chat LLM 流式输出）
- `workflowStreamStore.ts` → WorkflowStream（工作流执行流）

---

### 3.7 Tauri 命令

**位置**：`src-tauri/src/commands/stream_commands.rs`

```rust
use tauri::State;
use runtime_core_stream_types::*;
use runtime_stream_runtime::*;

#[tauri::command]
pub async fn workflow_stream_subscribe(
    bus: State<'_, Arc<dyn InProcessBus>>,
    topic: String,
    filter: Option<Filter>,
) -> Result<u64, String> {
    // 返回 subscription_id
    unimplemented!()
}

#[tauri::command]
pub async fn workflow_stream_unsubscribe(
    handle_id: u64,
) -> Result<(), String> {
    unimplemented!()
}

#[tauri::command]
pub async fn workflow_stream_query(
    bus: State<'_, Arc<dyn InProcessBus>>,
    topic: String,
    window: Window,
) -> Result<Vec<WorkflowStreamEvent<serde_json::Value>>, String> {
    unimplemented!()
}

#[tauri::command]
pub async fn workflow_stream_inject(
    bus: State<'_, Arc<dyn InProcessBus>>,
    topic: String,
    event: serde_json::Value,
) -> Result<(), String> {
    unimplemented!()
}
```

**两步注册（CLAUDE.md 禁区 #7）**：
1. `src-tauri/src/commands/mod.rs`：添加 `pub mod stream_commands;`
2. `src-tauri/src/lib.rs`：在 `generate_handler![]` 中注册 `workflow_stream_subscribe`、`workflow_stream_unsubscribe`、`workflow_stream_query`、`workflow_stream_inject`

**跨进程推送**：使用 Tauri 内置事件系统（`app_handle.emit` / `app_handle.listen`），不引入 SSE。

---

## 4. Subscription Protocol 详细规范

### 4.1 订阅生命周期状态机

```
                  build
                   │
                   ▼
        ┌──────────────────┐
        │     Pending      │ ←── 被订阅方未启动
        └──────────────────┘
                   │  source started
                   ▼
        ┌──────────────────┐
  ┌────▶│     Active       │ ←── 正在接收事件
  │     └──────────────────┘
  │                │ cancel / source end
  │                ▼
  │     ┌──────────────────┐
  └─────│    Completed     │ ←── 正常结束 / 取消
        └──────────────────┘
                   │  error
                   ▼
        ┌──────────────────┐
        │      Failed      │ ←── 异常终止
        └──────────────────┘
```

### 4.2 错误场景与处理

| 错误 | 行为 | 恢复策略 |
|------|------|---------|
| `stream_log` 写入失败 | 节点标记为 `Failed` 但不阻塞主路径 | 告警 metric +1；可重试 |
| `stream_log` 容量溢出 | ring buffer 滚动（保留最近 1000 条） | 可配置 |
| Subscription 失效 | 订阅方收到 `End` 事件 | 可选自动重订阅（默认否） |
| `query(window)` 超时 | 返回 partial 结果 + 错误码 | UI 提示缩小时间窗 |
| 运行期 `JsonSchema` 校验失败 | 节点输出事件被丢弃 | 错误日志 + 验证失败 metric |
| 跨工作流订阅方未启动 | 订阅进入 `Pending` 状态 | 等待启动后转 `Active` |
| 跨工作流 A↔B 循环订阅 | 启动前静态检测 | 报 `CyclicSubscription` 错误并拒绝 |

### 4.3 跨工作流订阅权限

**v1 简化**：工作流级 subscription 不做权限控制（同一用户的所有工作流可互订）。Sprint 4 后引入 ACL。

---

## 5. Migration Plan

### 5.1 引擎存储迁移（双写方案）

**Step 1**（Sprint 1.1）：在 `WorkflowExecution` struct 加 `stream_log: BTreeMap<String, Vec<...>>` 字段和 `next_stream_id: AtomicU64`。**不改变**现有 `results` 字段。

**Step 2**（Sprint 1.2）：在 `WorkEngine::update_node_status` 中调用 `StreamLogger::log_node_output` 双写：
```rust
pub async fn update_node_status(
    &self,
    execution_id: &str,
    node_id: &str,
    result: NodeResult,
) {
    // 现有逻辑写 results
    self.write_results(execution_id, node_id, &result);

    // 新增：写 stream_log
    let topic = format!("{}.outputs", node_id);
    self.stream_logger.log_node_output(
        self.executions.read().await.get(execution_id).unwrap(),
        node_id,
        &topic,
        serde_json::to_value(&result).unwrap(),
    ).await;
}
```

**Step 3**（Sprint 1.3）：单元测试验证 `results` 和 `stream_log` 一致性。

**Step 4**（Sprint 2）：前端默认从 `stream_log` 读（通过 `workflow_stream_query` IPC）。`results` 字段保留但 UI 不再使用。

**Step 5**（Sprint 4 末）：评估 GC `results` 字段。**前置条件**：所有 UI 切到 `stream_log` 至少 1 个迭代无回归。

### 5.2 兼容性保证

- 现有 28 节点 e2e 测试在 Sprint 1 末必须**全绿**
- 现有工作流 JSON 序列化格式不变
- 现有 Tracer / ExecutionHistory 面板不变
- 新增能力**只增不改**

### 5.3 性能基线

**Sprint 1 末必须做的基准**：
- 100+ 节点工作流单次执行：双写开销 < 10%（P99 延迟）
- 内存增长：每个执行 + ≤ 5%（stream_log 占用）
- 锁竞争：流操作不影响主执行路径的 P99

---

## 6. Test Matrix

| 测试类型 | 范围 | 数量目标 |
|---------|------|---------|
| 单元测试 | `stream_types` crate 各 trait | ≥ 30 个 |
| 单元测试 | `stream_runtime` InProcessBus | ≥ 20 个 |
| 集成测试 | WorkflowExecution 双写一致性 | ≥ 10 个 |
| 集成测试 | 锁顺序纪律（compile-time + runtime） | ≥ 5 个 |
| 集成测试 | 跨工作流订阅生命周期 | ≥ 8 个 |
| 集成测试 | 循环订阅静态检测 | ≥ 5 个 |
| 端到端 | 现有 28 节点工作流不回归 | 全部 |
| 端到端 | 反应式系统模板 demo | 1 个 |
| 端到端 | 可观测系统模板 demo | 1 个 |
| 性能 | 100+ 节点双写 P99 < 10% | 1 个基准 |

**Sprint 1 准入标准**：单元测试 100% 通过 + 现有 28 节点 e2e 全绿 + 双写 P99 < 10%。

---

## 7. Open Questions

1. **流持久化策略**：Sprint 1 默认 in-memory + ring buffer。Sprint 2 后根据需求决定全量持久化（SQLite BLOB？独立 log table？）。
2. **跨工作流订阅 ACL**：v1 简化（无权限），Sprint 4 引入。
3. **StreamDecl schema 演化**：v1 仅 `name + schema`，schema 变更后旧订阅如何处理？Sprint 3 引入 `schema_version` 字段。
4. **AI 辅助如何与流式视角结合**：AI 生成工作流时是生成 DAG 节点还是直接生成 stream pipeline？Sprint 3 后探索。
5. **类型生成工具选型**：`ts-rs` vs `specta` vs `tsify`？Sprint 1.1 决策。

---

## 8. Sprint 拆分

### Sprint 1：核心原语 + 引擎兼容层
**Duration**: 2 周
**Deliverables**:
- `runtime-core/stream_types` crate（含全部 trait）
- `runtime/stream_runtime` InProcessBus 默认实现
- 引擎 `WorkflowExecution` 双写
- 节点元数据自动推导（≤ 100 行）
- 单元测试 ≥ 50 个 + 现有 28 节点 e2e 全绿 + 性能基线
- i18n 11 语言齐备
- Tauri 命令两步注册

**验收**：
- 现有任何工作流执行后，stream_log 与 results 字段一致
- 性能 P99 增加 < 10%

### Sprint 2：可视化层 + 视角切换
**Duration**: 1.5 周
**Deliverables**:
- StreamInspector 实时面板
- StreamQueryBuilder（基础聚合）
- 现有工作流的 stream 视角切换（feature flag）
- workflowStreamStore（domain tier）

**验收**：
- 切到 stream 视角后能实时看到节点输出流
- 切回 DAG 视角无回归

### Sprint 3：跨边界订阅 + 决策点 first-class
**Duration**: 2 周
**Deliverables**:
- 工作流级 subscription
- 决策点角色自动识别（`deriveRolesFromNodeType`）
- 反应式系统模板 demo（事件源 → 决策 → 副作用）
- 跨工作流订阅静态检测

**验收**：
- 模板 demo 跑通
- 循环订阅被静态检测拦截

### Sprint 4：观测原语深化 + 性能优化
**Duration**: 1.5 周
**Deliverables**:
- 流 metric（吞吐、延迟、错误率）
- StreamQueryBuilder 扩展（filter / aggregate 增强）
- 可观测系统模板 demo
- 评估 `results` 字段 GC（前置条件：UI 全切到 stream_log 至少 1 迭代）

**验收**：
- 模板 demo 跑通
- 性能无回归

---

## 9. References

- 设计文档：[stream-architecture.md](../designs/stream-architecture.md)
- 引擎源码：`src-tauri/crates/rt-workflow/src/work_engine/engine.rs`
- 节点基类：`src/components/workflow/Nodes/BaseNode.tsx`
- CLAUDE.md：项目规范源
- 现有 ChatStream Store：`src/stores/domain/streamStore.ts`（命名参照）

---

## 10. Changelog

- **2026-06-06**: 初稿，待评审
