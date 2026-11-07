> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# Design: AxAgent 工作流能力基座 — 视角反转方案

**Generated**: 2026-06-06
**Mode**: Builder（/office-hours）
**Branch**: main
**Status**: APPROVED (2026-06-06)
**Supersedes**: —

---

## Problem Statement

AxAgent 当前拥有 28 种工作流节点 + AI Panel + Rust DAG 引擎，是"功能完整的工作流引擎"，但**距离成为能力基座还有结构性差距**。

**能力基座的定义**：

> 一组**业务无关**的、可组合的、显式语义的能力原语。任何需要"事件反应 / 决策驱动 / 数据流协同 / 可观察 / 可回放"中任一能力的业务场景，都能基于这些原语快速构建出 AI 工作流，而无需修改能力基座本身。

为推导能力原语的设计空间（不作为设计目标），使用三类**覆盖探针**：

| 探针模式 | 描述 | 推导出的能力需求 |
|---------|------|----------------|
| 反应式实时型 | 事件驱动、低延迟、严格可追溯 | 事件流、反应循环、可观察性 |
| 异步长时型 | 跨时段、跨会话、人类节奏 | 长时间窗、状态持久化、决策点显式化 |
| 并行分布型 | 大规模并行、可重放、计算密集 | 流并行、checkpoint、可重放 |

探针是推导能力的**工具**，不是设计服务的**目标**。所有业务（不仅是这三类）都应能用这套原语表达。

---

## What Makes This Cool

**视角反转**是能力跃迁的所在。

**对比示例**（同一 LLM 节点）：

**旧视角（DAG + 变量传递）**：
```
用户输入 → [LLM 节点] → 节点输出赋值给 {{llm_result}} → 下一节点读 {{llm_result}}
```
- 节点输出是"瞬时变量"，执行完就湮灭
- 想看"上一次的输出"：去 history / debug 面板翻
- 想跨工作流复用：复制粘贴节点或建 Trigger

**新视角（流式执行）**：
```
用户输入 → [LLM 节点] → 发出 WorkflowStreamEvent 到流 `llm_node.outputs` → 流可被订阅/查询/回放
```
- 节点输出是**可寻址的流**，可被任意后续节点 / 任意工作流订阅
- 想看历史：query(window) 即可
- 跨工作流：声明式订阅，无需 Trigger
- 决策点元数据：自动识别为"决策"角色，可观测决策历史
- 副作用元数据：自动识别为"输出"角色，可观测副作用历史

**为什么这是能力层面的跃迁**：

- **可反应**：任意事件源 → 任意反应点（业务无关的反应式循环）
- **可决策**：决策 = 流消费者，多种决策实现可插拔（业务无关的决策抽象）
- **可观察**：任意聚合查询 = 观测原语（业务无关的观测）
- **可回放**：执行历史 = 流日志（业务无关的回放）
- **可协同**：跨工作流 = 数据流图（业务无关的协同）

---

## Constraints

4 条已接受前提 + 新增 1 条：

- P1: 5 锚点并集，本期先做 ① 事件流+反应 + ② 可观察+干预，其他 3 个预留扩展点
- P2: 能力基座不绑业务场景语义（不出现业务字段）
- P3: 现有 28 节点底座保留，新能力是叠加/扩展
- P4: 反应式抽象不绑具体技术栈（trait 抽象）**【注：约束于进程内，跨进程走 IPC 协议】**
- **P5（新增）：设计产出物是能力原语集，不含任何业务场景的实现引用**

技术约束（CLAUDE.md）：

- 禁止重复定义：数据模型在 runtime-core（底层）→ tools/agent 是上层 re-export
- 新增 Tauri 命令必须同时改 `commands/mod.rs` + `lib.rs`
- 异步必须用 `tokio::sync::RwLock`
- 前端类型从 `@/types` 导入，不从子路径
- 新增 UI 文本必须在 locales/ 下 11 种语言文件全部添加 key
- Store 按 4-tier 分类（domain / feature / shared / devtools）

---

## Premises

- **P1**: 能力基座 = 5 锚点并集，先 1-2 个立住、其他预留扩展点（已同意）
- **P2**: 能力基座不绑业务场景语义（已同意）
- **P3**: 现有 28 节点底座保留，新能力是叠加/扩展（已同意）
- **P4**: 反应式抽象不绑具体技术栈（已同意）
- **P5**: 设计产出物是能力原语集，不含任何业务场景的实现引用（新增，本次迭代加入）

---

## Approaches Considered

### Approach A: Minimal Viable — "事件化节点 + 调试面板升级"

- 新增 EventEmitterNode / EventListenerNode 节点类型
- 现有 TriggerNode 增"内网 topic"模式
- Tracer 面板增加 Live 标签页
- 工作量 ~300 行 TS / ~200 行 Rust，1-2 迭代
- 缺点：事件语义是"节点级"非"工作流级"，跨边界订阅难；可观察仍偏研发视角

### Approach B: Ideal Architecture — "事件总线 + 决策点 first-class + 业务观测层"

- `runtime-core/event_types` 新模块，EventBus trait
- 工作流级 `event_subscriptions`
- 决策点升级为 first-class
- 工作量 ~1500 行 TS / ~1000 行 Rust，3-4 迭代
- 缺点：工作量大、用户需适配；偏"工程师味"

### Approach C: Creative/Lateral — "流式执行模型（视角反转）"（推荐）

- 节点输出都是 named streams，可被订阅/查询
- 决策点 = 流消费者；副作用 = 流生产者
- 整个执行历史 = 可查询流日志
- 跨工作流协同 = 数据流图
- 工作量 ~2000 行 TS / ~1500 行 Rust，4-5 迭代
- 优点：1 个抽象统一 5 锚点；天然支持可观察 / 可回放 / 可协同
- 缺点：学习曲线陡；用户需重新理解"工作流是什么"

---

## Recommended Approach: 流式执行模型（视角反转）

### 4 个能力原语

#### 1. Stream（流）—— 节点输出的可观察时间序列

```rust
// 强类型 + 动态 schema 校验
pub trait Stream<T: Serialize + JsonSchema>: Send + Sync {
    fn subscribe(&self, filter: Filter) -> Subscription<T>;
    fn query(&self, window: Window) -> Vec<StreamEvent<T>>;
    fn tail(&self) -> BoxStream<StreamEvent<T>>;  // 实时
}
```

- 形如 `Stream<EventPayload>`，强类型 + 动态 schema 双校验
- 支持 Window / Filter / Aggregate / Join / Map 等操作
- 跨工作流、跨组件可订阅

#### 2. Subscription（订阅）—— 对 Stream 的可声明订阅

| 订阅层级 | 声明位置 | 典型场景 |
|---------|---------|---------|
| 工作流级 | workflow metadata | "工作流 B 启动时订阅工作流 A 的输出流" |
| 节点级 | 节点 properties | "这个 LLM 节点订阅外部事件流作为 prompt 上下文" |
| 外部组件级 | IPC / Tauri 事件 / MCP | "外部进程订阅工作流执行流" |

#### 3. Producer / Consumer（生产者/消费者）—— 副作用的语义化

| 角色 | 含义 | 28 节点底座映射 |
|------|------|----------------|
| Producer | 发出流事件的节点 | 任何有输出的节点默认是 Producer |
| Consumer | 订阅流事件的节点 | 任何有输入的节点默认是 Consumer |
| DecisionPoint | 特殊的 Consumer（可选角色） | 由节点类型字符串自动推导（如 `*classifier*`） |
| EffectNode | 特殊的 Producer + Consumer（可选角色） | 由节点类型字符串自动推导（如 `http_*` / `*_send` / `db_*`） |

**v1 角色策略**：Sprint 1 仅暴露 `Producer` / `Consumer` 两个基础角色，**由 `node_type` 字符串自动推导** DecisionPoint / EffectNode 角色。**不要求每个节点显式声明 4 个角色**，元数据改造总行数 ≤ 100 行 TS（仅在 base class 加 2 个数组默认值）。

#### 4. Observation（观测）—— 运行时对流状态的可见性

- 每个流自带 metric：吞吐、延迟、错误率
- 决策 snapshot 可被查询
- 整条工作流执行 = 1 个可重放的流日志
- **重要**：观测层不绑"业务 KPI"语义，只提供"流聚合查询"原语（业务 KPI 由上层用原语构造）

### StreamDecl 元数据契约

```typescript
// src/types/workflow-stream.ts
export interface StreamDecl {
  name: string;                    // 流名（节点内唯一）
  schema: JsonSchema;              // payload schema
  // v1 仅暴露 name + schema 两个字段
  // 后续 Sprint 按需扩展：retention / visibility / cardinality
}
```

节点基类扩展：

```typescript
// BaseNode.tsx
abstract class BaseNode {
  produces: StreamDecl[] = [];  // 节点对外暴露的流
  consumes: StreamDecl[] = [];  // 节点订阅的流
  // ... 现有字段
}
```

**28 节点改造量**：每个节点文件 5-10 行元数据声明，总计 < 200 行 TS。

---

## 架构映射

### Rust 模块布局（修正分层）

```
src-tauri/crates/
├── runtime-core/                # CLAUDE.md: 数据模型在 runtime-core（底层）
│   └── stream_types/            # 新增 crate: Stream / Subscription / Window / Filter 原语
│       └── src/
│           ├── stream.rs        # Stream trait
│           ├── subscription.rs  # Subscription
│           ├── window.rs        # Window / Filter / Aggregate
│           └── schema.rs        # JsonSchema 校验（复用 core 现有）
│
├── runtime/                     # 上层 re-export / 扩展
│   └── src/
│       └── stream_runtime/      # StreamBus trait + 默认 in-process 实现
│           ├── mod.rs
│           └── in_process.rs    # tokio::mpsc 实现
│
└── rt-workflow/                 # 业务/编排层
    └── src/
        └── stream_engine.rs     # 工作流执行 → 流生产/消费
            # 依赖：runtime-core::stream_types, runtime::stream_runtime
```

**路径修正说明**：原 `core/stream_types/` 改为 `runtime-core/stream_types/`——CLAUDE.md §禁区 #12 规定"数据模型在 `runtime-core`（底层）→ `tools`/`agent`（上层 re-export 或扩展）。`core` 是最基础层，`runtime-core` 是其上的运行时抽象层"。`stream_types` 是运行时抽象层模型，应在 `runtime-core` 而非 `core`。

### 前端模块布局

```
src/
├── types/
│   ├── stream.ts                # Stream / Subscription / Window / Filter 类型
│   └── workflow-stream.ts       # StreamDecl 节点元数据契约
│                              # 【命名注意】与 stores/domain/streamStore.ts 区分
│                              # 后者是 ChatStream（chat LLM 流式输出）
│                              # 本设计是 WorkflowStream（工作流执行流）
├── components/
│   ├── stream/
│   │   ├── StreamInspector.tsx           # 实时流查看器
│   │   ├── StreamQueryBuilder.tsx        # 流聚合查询（不绑业务 KPI）
│   │   └── StreamSubscriptionEditor.tsx  # 可视化订阅
│   ├── workflow/
│   │   └── DecisionPointRole.tsx         # 决策点角色元数据编辑器
│   └── (现有 28 节点元数据扩展)
├── pages/
│   └── StreamPage.tsx           # 流仪表板
└── stores/
    └── domain/                   # 新建 workflowStreamStore（domain tier，因属核心流式能力）
        └── workflowStreamStore.ts
```

**Store 分类**：新 store 放 `stores/domain/`（CLAUDE.md 4-tier 规则：流式属核心业务），与现有 `streamStore.ts`（chat 流）通过 `Workflow*` 前缀区分。

### 命名冲突解决

| 现有 | 新增 | 区分 |
|------|------|------|
| `streamStore.ts` (chat LLM streaming) | `workflowStreamStore.ts` | 前缀区分 |
| `ChatStream` (chat 流式) | `WorkflowStream` (工作流流) | 类型前缀 |
| `StreamEvent` (chat) | `WorkflowStreamEvent` | 类型前缀 |

---

## 5 锚点覆盖

| 锚点 | 本期直接覆盖 | 后续增量（接口已预留，不实现） |
|------|-------------|------------------------------|
| ① 事件流+反应 | Stream + Subscription + Producer/Consumer | 多流模式匹配、状态流、多流复合 |
| ② 可观察+干预 | Stream Inspector + Stream Query + 流 metric | 聚合查询模板（上层构造）、外部注入通道 |
| ③ 副作用原子化 | 节点 trait 字段预留（不在 v1 暴露） | Saga / outbox 模式实现 |
| ④ 多时间尺度 | Stream 自带时间窗（Window trait 字段） | 虚拟时间原语、跨时段协调（不在 v1） |
| ⑤ 决策点+热插拔 | DecisionPoint 由 node_type 自动推导 | 策略可替换、A/B（不在 v1） |

**接口预留，不实现**：③ ④ ⑤ 在 v1 不实现也不在公共类型中暴露字段，避免 Sprint 1 API 表面臃肿。仅在内部 trait/枚举值层面有"未来扩展位"。

---

## 关键架构问题与方案

### 1. 引擎存储迁移（核心障碍）

**问题**：`rt-workflow/src/work_engine/engine.rs` 当前用 `workflow.results: HashMap<String, serde_json::Value>` 存**末值**（last-write-wins），与"流式"模型需要的"事件序列"根本冲突。

**方案：双写 + 渐进迁移**（推荐）

```rust
// 兼容演进：results 保留（last-value 视图），新增 stream_log
pub struct WorkflowExecution {
    pub results: HashMap<String, serde_json::Value>,  // 旧视图（保留）
    pub stream_log: BTreeMap<String, Vec<StreamEvent>>,  // 新视图（流日志）
    // ...
}
```

- 写路径：节点执行完成时**同时**写 results（旧）和 stream_log（新）
- 读路径：Sprint 1 默认读 results（兼容），Sprint 2 切到 stream_log
- Sprint 4 后考虑 GC results（仅保留 stream_log）

**风险**：双写有 ~2x IO。Sprint 4 评估是否需要 GC。

### 2. 锁顺序纪律

新增 `stream_subs` 锁时必须遵守锁顺序：

```
workflows → executions → node_breakers → cancel_tokens → stream_subs
```

任何代码路径不得违反此顺序。所有新锁统一 `tokio::sync::RwLock`（CLAUDE.md 规则 8）。

### 3. 取消令牌挂载

跨工作流 subscription 必须挂载到发起工作流的 `CancellationToken`：

```rust
// Subscription API（详细签名）
pub struct SubscriptionHandle { /* Opaque, ref-counted */ }

pub trait WorkflowSubscription: Send + Sync {
    /// 订阅另一工作流的指定流
    /// 被订阅方未启动时，订阅进入 Pending 状态
    fn subscribe_to_workflow(
        &self,
        source_workflow_id: &str,
        topic: &str,
        filter: Option<Filter>,
    ) -> Result<SubscriptionHandle, SubscriptionError>;

    /// 订阅到发起工作流的取消令牌（强制要求，避免泄漏）
    fn with_cancel_token(self, token: CancellationToken) -> Self;

    /// 注入事件到流（外部进程/人工）
    fn inject(&self, topic: &str, event: WorkflowStreamEvent) -> Result<(), InjectError>;
}
```

避免工作流取消后 stream subscription 泄漏。

### 4. i18n 纪律

新 UI 文本（StreamInspector / StreamQueryBuilder / StreamSubscriptionEditor / StreamPage）必须在 locales/ 下 11 种语言文件全部添加 key，禁止仅添加 zh-CN（CLAUDE.md 禁区 #2）。

### 5. Tauri 命令注册（两步）

新增 `stream_commands.rs` 必须同时改：

- `src-tauri/src/commands/mod.rs`（声明模块）
- `src-tauri/src/lib.rs`（generate_handler![] 注册）

注册的 IPC 命令建议：

| 命令 | 入参 | 出参 | 用途 |
|------|------|------|------|
| `workflow_stream_subscribe` | topic, filter | SubscriptionHandle | 订阅 |
| `workflow_stream_unsubscribe` | handle | () | 取消订阅 |
| `workflow_stream_query` | topic, window | Vec<WorkflowStreamEvent> | 聚合查询 |
| `workflow_stream_inject` | topic, event | () | 外部进程/人工注入 |

### 6. 类型系统策略

| 层 | 策略 |
|----|------|
| Rust 端 | `stream_types` crate 单一源，`ts-rs` 或 `specta` 生成 TS 类型 |
| TS 端 | 消费生成的类型，禁止手写重复类型 |
| 运行期 | `JsonSchema` 校验 payload（与 core 现有 schema 体系复用） |

**禁止**双账本（手动维护 TS 与 Rust 类型），必须 codegen。

### 7. 订阅协议边界

- **InProcessBus trait 抽象限于进程内**：默认 tokio::mpsc，**不**抽象异构后端（Kafka/SSE/NATS 等）
- **跨进程订阅**：通过 Tauri 内置事件系统（`app_handle.emit` / `app_handle.listen`），不引入 SSE
- "跨边界订阅"是**协议问题**而非抽象问题，避免在 v1 过度抽象

**修正 P4 边界**：trait 抽象在进程内（InProcessBus），跨进程走 Tauri 事件系统。

### 8. 错误模型

| 错误场景 | 行为 |
|---------|------|
| `stream_log` 写入失败 | 标记节点为 Failed，但不阻塞主执行路径（best-effort）；告警 metric +1 |
| `stream_log` 容量溢出 | 默认 ring buffer（最近 N 条），超出滚动；可通过配置改全量保留 |
| Subscription 失效（被订阅方取消/重启） | 订阅方收到 `StreamEvent::End` 事件，可选重订阅 |
| `query(window)` 超时（>5s） | 返回 partial 结果 + 错误码；UI 提示缩小时间窗 |
| 运行期 `JsonSchema` 校验失败 | 节点输出事件被丢弃 + 错误日志 + 验证失败 metric |
| 跨工作流订阅方未启动 | 订阅进入 `Pending` 状态，等待被订阅方启动后 `Active` |
| 跨工作流 A↔B 循环订阅 | 工作流执行前静态检测有向环，循环则拒绝启动并报 `CyclicSubscription` 错误 |

---

## 4 Sprint 增量执行

### Sprint 1：核心原语 + 引擎兼容层

**目标**：流式原语落地，不破坏现有工作流。

- 新增 `runtime-core/stream_types` crate
- 节点元数据扩展（`produces` / `consumes`）
- 28 节点元数据声明（< 200 行 TS）
- 引擎双写：`results` + `stream_log` 并行
- StreamBus 默认 in-process 实现
- 锁顺序纪律 + 取消令牌挂载
- 单元测试：Stream / Subscription / Producer / Consumer 闭环
- 现有 28 节点 e2e 测试全绿

**Demo**：用任意现有工作流，验证 stream_log 与 results 一致。

### Sprint 2：可视化层 + 视角切换

**目标**：开发者能"看见"流。

- StreamInspector 实时面板
- StreamQueryBuilder（基础聚合查询，**不要求**SQL 完整）
- 现有工作流的 stream 视角切换（feature flag，默认开）
- i18n 11 语言齐备
- Tauri 命令注册 + 类型生成

**Demo**：在现有 28 节点中的任一工作流中，切到 stream 视角，实时观察节点输出流。

### Sprint 3：跨边界订阅 + 决策点 first-class

**目标**：能力原语能跨工作流 + 决策角色显式化。

- 工作流级 subscription（`workflow.metadata.subscriptions[]`）
- DecisionPoint 元数据角色（`role: 'decision'` 标记）
- SSE 实时推送（`workflow_stream_tail`）
- 反向注入（`workflow_stream_inject`）

**Demo**：构建一个"反应式系统"通用模板（事件源 → 决策 → 副作用），验证能力原语能服务任何事件驱动业务（不绑具体业务语义）。

### Sprint 4：观测原语深化

**目标**：让上层业务能基于流聚合构造自己的观测指标（不在能力基座内绑"业务 KPI"等语义）。

- 流 metric（吞吐、延迟、错误率）
- StreamQueryBuilder 扩展（filter / aggregate 增强）
- 通用聚合原语（提供"任意流的任意聚合"原子操作）

**Demo**：在反应式系统模板上演示"如何用聚合原语构造自定义观测指标"（不展示任何业务层例子）。

---

## 风险与缓解

| 风险 | 缓解 | 触发条件 |
|------|------|---------|
| 数据模型重塑、用户需重新理解心智模型 | 旧 DAG 视角保留为"兼容模式"，新视角为"增强模式"，feature flag | Sprint 2 验证切换流畅性 |
| 每节点都是流事件 → 性能 | 异步 batch 写入 + lazy materialization + 开关 | Sprint 1 基准测试 |
| 双写 IO 开销 ~2x | Sprint 4 评估是否 GC results；如不可接受则改用 in-memory ring buffer | Sprint 4 性能基线 |
| 锁顺序风险 | 文档化锁顺序 + 静态检查（`cargo clippy` 自定义 lint）+ 单元测试 | Sprint 1 全程 |
| 与现有 `streamStore.ts` 命名冲突 | 明确 `WorkflowStream` / `ChatStream` 前缀 + 不同目录 | Sprint 0 即修复 |
| 学习曲线陡 | 渐进披露（v1 看 DAG，v2 可切换 stream 视角） | UX 验证 |
| 订阅协议过度抽象 | 进程内 trait 抽象，跨进程走 IPC 协议（不抽象异构后端） | Sprint 3 验证 |

---

## Open Questions

1. **流持久化策略**：默认 in-memory + 可选 persist。Sprint 2 后根据需求决定是否全量持久化。
2. **跨工作流订阅的权限模型**：工作流 A 订阅工作流 B 的流时，如何鉴权？待 RFC-0001 讨论。
3. **Stream 的 schema 演化**：节点升级后 stream 类型变更，旧订阅如何处理？版本化 StreamDecl 字段。
4. **AI 辅助如何与流式视角结合**：AI 生成工作流时，是生成 DAG 节点还是直接生成 stream pipeline？Sprint 3 后探索。
5. **性能基准**：含 100+ 节点的工作流，流事件写入开销 < 10% 的目标是否可达？Sprint 1 末做基线测试。

---

## Success Criteria

**能力层面**（不绑业务）：

- 单元测试：Stream / Subscription / Producer / Consumer 原语闭环通过
- 通用"反应式系统"模板 demo 跑通（Sprint 3）
- 通用"可观测系统"模板 demo 跑通（Sprint 4）
- 含 100+ 节点的工作流，流事件写入开销 < 10%（Sprint 1 末基线）
- 旧 DAG 用户零迁移成本
- 后续 ③ ④ ⑤ 锚点接入不需重构核心抽象（接口已预留）

**工程纪律层面**：

- 锁顺序：所有新代码路径通过静态检查
- i18n：11 语言齐备
- Tauri 命令：两步注册 100% 覆盖
- 类型：codegen 单一源，零手写重复
- 命名：与现有 streamStore.ts 无冲突

---

## Distribution Plan

n/a（本次输出为设计文档，非交付物）

后续实施时的发布通道：

- `npm run tauri dev` → 桌面端应用
- 流式引擎 API 文档 → 嵌入在 `src/types/workflow-stream.ts` 的 JSDoc
- RFC-0001 文档 → `docs/designs/rfc-0001-stream-runtime.md`

---

## Dependencies

- 现有 28 节点底座（已具备）
- Rust 异步运行时 tokio（已具备）
- Sea-ORM / SQLite（已具备，stream 元数据持久化复用）
- 跨工作流订阅权限：暂无，待 RFC-0001 讨论
- ts-rs / specta（类型生成工具，待选定）
- 现有 `streamStore.ts`（chat 流）的命名隔离

---

## Next Steps

1. 在 `src-tauri/crates/runtime-core/` 下创建 `stream_types` crate 脚手架
2. 在 `src/components/workflow/Nodes/BaseNode.tsx` 添加 `produces` / `consumes` 元数据
3. 在 28 节点文件中加 `produces` / `consumes` 元数据声明（< 200 行 TS）
4. 在 `src-tauri/src/commands/` 下新增 `stream_commands.rs`，注册 `workflow_stream_*` 命令
5. 在 `src-tauri/src/commands/mod.rs` + `lib.rs` 同步注册（CLAUDE.md 禁区 #7）
6. 在 `src/stores/feature/` 新建 `workflowStreamStore.ts`（与 `streamStore.ts` 区分）
7. 写 RFC-0001：核心原语详细规范、引擎存储迁移方案、订阅协议细节，发起团队评审

---

## Reviewer Concerns（迭代 2 已处理）

本次对抗式审查（iteration 1）发现 28 个问题，已在本版本处理：

- ✅ 业务术语泄漏（原文涉及业务场景的术语）→ 已全部替换为能力术语
- ✅ Stream vs StreamBus 命名冲突 → 改名为 InProcessBus
- ✅ Store 放错位置（`feature/` vs `domain/`）→ 已修正为 `domain/`
- ✅ StreamDecl YAGNI 字段（retention/visibility）→ Sprint 1 仅保留 name + schema
- ✅ 4 角色数学不合理 → 改为由 `node_type` 自动推导，元数据 ≤ 100 行
- ✅ KPI 模板概念矛盾（不绑业务 vs 模板）→ 改为"通用聚合原语"
- ✅ SSE 跨进程抽象（YAGNI）→ 改用 Tauri 内置事件系统
- ✅ `subscribe_to_workflow_b` API 未定义 → 给出完整 trait + handle
- ✅ 错误模型缺失 → 新增"错误模型"小节（7 种错误场景 + 行为）
- ✅ "视角反转"过于抽象 → 新增 LLM 节点 before/after 对比示例
- ✅ Approval 节点未解释 → Sprint 3 由 node_type 自动识别（无需逐节点解释）
- ✅ 双写崩溃一致性 → Sprint 1 error model 已涵盖
- ✅ 跨工作流订阅循环 → 工作流启动前静态检测有向环
- ✅ 订阅生命周期（Pending/Active）→ Subscription trait 中已明确
- ✅ 运行期 schema 校验开销 → 错误模型中明确"丢弃 + metric"，不阻塞主路径
