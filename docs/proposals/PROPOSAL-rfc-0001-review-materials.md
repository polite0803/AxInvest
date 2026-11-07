> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# RFC-0001 评审材料

**配套文档**：
- 设计：[stream-architecture.md](./stream-architecture.md)（APPROVED）
- 规范：[rfc-0001-stream-runtime.md](./rfc-0001-stream-runtime.md)（DRAFT）
- 评审指南：[rfc-0001-review-guide.md](./rfc-0001-review-guide.md）
- Sprint 1 实施计划：[2026-06-06-stream-runtime-sprint-1.md](../plans/2026-06-06-stream-runtime-sprint-1.md)

**目标读者**：AxAgent 核心开发团队（Rust + TS 双向栈）
**配套使用**：评审会议前 1 天发给参会者，与 `rfc-0001-review-guide.md` 配套阅读

---

## 目录

1. [关键架构纠偏（必读）](#1-关键架构纠偏必读)
2. [LLMNode 实际改造示例](#2-llmnode-实际改造示例)
3. [LlmExecutor 实际改造示例](#3-llmexecutor-实际改造示例)
4. [BaseNode.tsx before/after 对比](#4-basenodetsx-beforeafter-对比)
5. [28 节点改造总览矩阵](#5-28-节点改造总览矩阵)
6. [迁移路径时间线](#6-迁移路径时间线)
7. [评审会前自查清单](#7-评审会前自查清单)
8. [FAQ（10 个最可能问题）](#8-faq10-个最可能问题)

---

## 1. 关键架构纠偏（必读）

> **这是评审时最容易走偏的点**：评审者通常基于"业务类项目经验"想象改造路径，但 AxAgent 的实际结构与典型 Web 后端不同。

### 1.1 节点元数据不在前端类，而在 Rust trait

**错误想象**（基于典型 Node.js / Python 项目）：
```typescript
// ❌ 这是基于典型类组件的想象
class LLMNode extends BaseNode {
  produces: StreamDecl[] = [
    { name: 'response', schema: { type: 'string' } },
  ];
}
```

**实际结构**：
```typescript
// ✅ 实际：BaseNode.tsx 是 React 函数组件，不是类
// src/components/workflow/Nodes/BaseNode.tsx
const BaseNodeComponent: React.FC<NodeProps<BaseNodeData>> = ({ data, selected }) => {
  // ... 纯 UI 渲染 ...
  return <div>...</div>;
};
```

**真实元数据载体**：Rust 端 `NodeExecutorTrait`
```rust
// ✅ 实际：src-tauri/crates/rt-workflow/src/work_engine/node_executor_trait.rs
#[async_trait]
pub trait NodeExecutor: Send + Sync {
    fn node_type(&self) -> &'static str;
    // ... 现有方法 ...
}
```

> **改造影响**：RFC §3.4 中"≤ 100 行 BaseNode 改造" 的设计**自动满足**——因为无需改 BaseNode.tsx（元数据在 Rust 端），Sprint 1 任务 T1.12 在 Rust trait 加默认方法 `stream_meta()`，**28 节点 0 改动**。

### 1.2 现有 28 节点实际数量是 30（包含 fallback + llm_resolve）

**统计位置**：[executors/mod.rs:1-29](../../src-tauri/crates/rt-workflow/src/work_engine/executors/mod.rs#L1-L29)

| 节点 | node_type() 返回 | 是否需要改造 | 备注 |
|------|-----------------|-------------|------|
| Agent | `"agent"` | ❌ 自动默认 | 通过 stream_meta() 默认实现拿到 roles |
| Aggregator | `"aggregator"` | ❌ | |
| Approval | `"approval"` | ❌ | 自动识别为 decision_point |
| Code | `"code"` | ❌ | |
| Condition | `"condition"` | ❌ | 自动识别为 decision_point |
| DataTransformer | `"data_transformer"` | ❌ | |
| DatabaseQuery | `"db_query"` | ❌ | 自动识别为 effect_node |
| Debate | `"debate"` | ❌ | |
| Delay | `"delay"` | ❌ | |
| DocumentParser | `"document_parser"` | ❌ | |
| Email | `"email_send"` | ❌ | 自动识别为 effect_node |
| End | `"end"` | ❌ | |
| Fallback | `"fallback"` | ❌ | 内部使用 |
| FileOperation | `"file_operation"` | ❌ | |
| HttpRequest | `"http_request"` | ❌ | 自动识别为 effect_node |
| LLM | `"llm"` | ❌ | 详细见 §3 |
| LlmClassifier | `"llm_classifier"` | ❌ | 自动识别为 decision_point |
| Logging | `"logging"` | ❌ | |
| Loop | `"loop"` | ❌ | |
| Merge | `"merge"` | ❌ | |
| Notification | `"notification_send"` | ❌ | 自动识别为 effect_node |
| Parallel | `"parallel"` | ❌ | |
| SubWorkflow | `"sub_workflow"` | ❌ | |
| Switch | `"switch"` | ❌ | 自动识别为 decision_point |
| Tool | `"tool"` | ❌ | |
| Trigger | `"trigger"` | ❌ | |
| Validation | `"validation"` | ❌ | |
| VectorRetrieve | `"vector_retrieve"` | ❌ | |
| WebhookSend | `"webhook_send"` | ❌ | 自动识别为 effect_node |

**结论**：**30 个 executor 全部 0 改动**（用 `stream_meta()` 默认方法自动推导 roles）。

### 1.3 节点类型字符串的"约定"很脆弱

`derive_roles_from_node_type` 用正则匹配关键字（`classifier` / `condition` / `http_` / `email` / `webhook` 等）。**这意味着**：未来新增节点时，**节点类型命名必须遵守约定**，否则元数据会错。

**风险**：
- 假设有人新增 `decision_aggregator`（用 `aggregator` 命名），会**漏掉** decision_point 角色
- 假设有人新增 `trigger_send_email`，会被识别为 effect_node，但其实是 trigger

**Sprint 1 缓解措施**：
- 在 `executors/mod.rs` 顶部加 doc 警告
- Sprint 2 时增加 `node_type_contracts.md` 强制约束
- Sprint 3 后可改为 `enum NodeKind` 替代字符串

> **评审讨论点**：是否在 Sprint 1 就把 `node_type` 改为 `enum`？建议**否**（YAGNI，先用字符串+约定）。

---

## 2. LLMNode 实际改造示例

### 2.1 改造前（实际文件：[LLMNode.tsx](../../src/components/workflow/Nodes/LLMNode.tsx)）

```typescript
// src/components/workflow/Nodes/LLMNode.tsx (前 80 行)
const LLMNodeComponent: React.FC<NodeProps<LLMNodeData>> = ({
  data,
  selected,
}) => {
  // 纯 UI 渲染：图标、温度、token 显示
  // ...
  return <div>...</div>;
};

export default memo(LLMNodeComponent);
```

**改造前总行数**：约 180 行（**纯 UI 组件，无元数据声明**）

### 2.2 改造后（RFC Sprint 1 范围）

**零改动**！

```typescript
// src/components/workflow/Nodes/LLMNode.tsx (无需修改)
// LLMNode 仍是纯 UI 组件
// 元数据自动从 Rust 端 node_type() = "llm" 推导
```

### 2.3 Sprint 2 才需要的扩展（不在 RFC Sprint 1 范围）

Sprint 2 后如果 LLMNode 需要声明"产生 response 流 + 订阅 prompt 模板流"：

```typescript
// src/components/workflow/Nodes/LLMNode.tsx (Sprint 2+ 可选扩展)
import { useNodeStreamMeta } from "@/hooks/useNodeStreamMeta";
import type { StreamDecl } from "@/types/workflow-stream";

const LLMNodeComponent: React.FC<NodeProps<LLMNodeData>> = ({ data }) => {
  // Sprint 2+ 可选：UI 组件调用 hook 拉取 Rust 端元数据并展示
  // （元数据本身仍在 Rust 端，UI 只是消费）
  const meta = useNodeStreamMeta(data.nodeType);
  // meta = { nodeType: "llm", roles: ["producer", "consumer", "effect_node"] }

  return (
    <div>
      {/* 现有 UI */}
      {meta && meta.roles.includes("effect_node") && <Tag color="orange">有副作用</Tag>}
    </div>
  );
};
```

> **关键**：Sprint 1 阶段 `useNodeStreamMeta` 还不存在（属于 Sprint 2 StreamInspector 范围）。Sprint 1 的元数据完全在 Rust 端。

### 2.4 文件总览

| 文件 | Sprint 1 改动 | 行数变化 |
|------|---------------|---------|
| `LLMNode.tsx` | ❌ 0 | 0 |
| 其他 27 个节点 `.tsx` | ❌ 0 | 0 |
| `BaseNode.tsx` | ❌ 0 | 0 |

**前端 Sprint 1 唯一新增**：`src/types/workflow-stream.ts`（ts-rs 生成的 re-export + 3 个手写辅助类型）

---

## 3. LlmExecutor 实际改造示例

### 3.1 改造前（实际文件：[llm_executor.rs](../../src-tauri/crates/rt-workflow/src/work_engine/executors/llm_executor.rs)）

```rust
// src-tauri/crates/rt-workflow/src/work_engine/executors/llm_executor.rs
#[async_trait]
impl NodeExecutorTrait for LlmExecutor {
    fn node_type(&self) -> &'static str {
        "llm"
    }

    async fn execute(
        &self,
        node: &WorkflowNode,
        context: &ExecutionState,
    ) -> Result<NodeOutput, NodeError> {
        // ... 现有 LLM 调用逻辑 ...
        Ok(NodeOutput {
            output: serde_json::to_value(&response)?,
            output_var: Some("llm_response".into()),
        })
    }
}
```

**改造前总行数**：约 350 行（含 LLM 解析、调用、错误处理）

### 3.2 改造后（RFC Sprint 1 范围）

```rust
// src-tauri/crates/rt-workflow/src/work_engine/executors/llm_executor.rs
// 唯一改动：在 impl 块中加 1 个方法（默认实现也 OK，无需重写）

#[async_trait]
impl NodeExecutorTrait for LlmExecutor {
    fn node_type(&self) -> &'static str {
        "llm"
    }

    // 🆕 Sprint 1 新增（5 行）—— 也可以不加（用 trait 默认实现）
    fn stream_meta(&self) -> crate::stream_meta::NodeStreamMeta {
        crate::stream_meta::derive_stream_meta("llm")
        // 默认返回：roles = [Producer, Consumer]（"llm" 不含 classifier/condition/switch/approval，
        //                                            也不以 http_/_send/db_/email/webhook 开头）
    }

    async fn execute(
        &self,
        node: &WorkflowNode,
        context: &ExecutionState,
    ) -> Result<NodeOutput, NodeError> {
        // ... 现有 LLM 调用逻辑完全不变 ...
        Ok(NodeOutput {
            output: serde_json::to_value(&response)?,
            output_var: Some("llm_response".into()),
        })
    }
}
```

> **更简方案**：LlmExecutor **完全不改**——`stream_meta()` 用 `NodeExecutor` trait 的默认实现（见 Sprint 1 任务 T1.12）。**30 个 executor 全部 0 改动**。

### 3.3 引擎层改造（WorkEngine.update_node_status）

唯一需要**主动**改的代码路径是 WorkEngine 的节点完成处理：

```rust
// src-tauri/crates/rt-workflow/src/work_engine/engine.rs
// 现有：
pub async fn update_node_status(
    &self,
    execution_id: &str,
    node_id: &str,
    result: NodeResult,
) {
    self.write_results(execution_id, node_id, &result);  // 现有
}

// Sprint 1 改造后：
pub async fn update_node_status(
    &self,
    execution_id: &str,
    node_id: &str,
    result: NodeResult,
) {
    // 现有：写 results
    self.write_results(execution_id, node_id, &result);

    // 🆕 Sprint 1 新增：写 stream_log
    let mut exec = self.executions.write().await.get_mut(execution_id).unwrap().clone();
    let topic = format!("{}.outputs", node_id);
    let logger = StreamLogger::new(self.stream_bus.clone());
    logger.log_node_output(
        &mut exec.results,
        &exec.workflow_id,
        exec.next_stream_id(),
        node_id,
        &topic,
        serde_json::to_value(&result).unwrap_or(serde_json::Value::Null),
    ).await
    .unwrap_or_else(|e| tracing::warn!("stream_log write failed: {}", e));
    // best-effort，不阻塞主路径
}
```

**总改动行数**：约 10 行（仅 engine.rs 1 个方法）

### 3.4 文件总览

| 文件 | Sprint 1 改动 | 行数变化 |
|------|---------------|---------|
| `node_executor_trait.rs` | 加 `stream_meta()` 默认方法 | +6 |
| `llm_executor.rs` | ❌ 0（用默认） | 0 |
| 其他 29 个 executor | ❌ 0 | 0 |
| `engine.rs::update_node_status` | 加 stream_log 双写 | +10 |
| `workflow_execution.rs` | 加 `stream_log` 字段 + 5 个方法 | +50 |
| `stream_types/*` | **全新模块** | +600 |
| `stream_commands.rs` | **全新模块** | +120 |

**总代码增量**：约 800 行（其中 600 行是 `stream_types` 模块，200 行是双写适配器/命令）

---

## 4. BaseNode.tsx before/after 对比

### 4.1 改造前

```typescript
// src/components/workflow/Nodes/BaseNode.tsx (实际 100 行)
import { useWorkEngineStore } from "@/stores/feature/workEngineStore";
import { theme } from "antd";
import React, { memo } from "react";
import { useTranslation } from "react-i18next";
import { Handle, type NodeProps, Position } from "reactflow";
import { NODE_TYPE_MAP } from "../types";

export interface BaseNodeData {
  id: string;
  type: string;
  title: string;
  description?: string;
  color: string;
  nodeType: string;
  enabled: boolean;
  validationState?: "error" | "warning";
  executionState?: "running" | "completed" | "failed" | "timeout" | "skipped" | "paused";
  parentId?: string;
}

const BaseNodeComponent: React.FC<NodeProps<BaseNodeData>> = ({ data, selected }) => {
  // 纯 UI 渲染
  // ...
  return <div>...</div>;
};

export default memo(BaseNodeComponent);
```

### 4.2 改造后（RFC Sprint 1）

```typescript
// src/components/workflow/Nodes/BaseNode.tsx
// ⚠️ 唯一改动：增加一个可选 import（仅当使用 t() 时）

import { useWorkEngineStore } from "@/stores/feature/workEngineStore";
import { theme } from "antd";
import React, { memo } from "react";
import { useTranslation } from "react-i18next";
import { Handle, type NodeProps, Position } from "reactflow";
import { NODE_TYPE_MAP } from "../types";

// ... BaseNodeData 不变 ...

const BaseNodeComponent: React.FC<NodeProps<BaseNodeData>> = ({ data, selected }) => {
  const { t } = useTranslation();  // 已有
  // ... 现有 UI 代码 0 改动 ...
  return <div>...</div>;
};

export default memo(BaseNodeComponent);
```

> **零实质改动**！Sprint 1 不需要改 BaseNode.tsx。

### 4.3 Sprint 2+ 可选扩展

如果 Sprint 2 之后需要让 BaseNode 显示 stream 角色徽章：

```typescript
// Sprint 2+ 可选扩展（不在 Sprint 1 范围）
import { useNodeStreamMeta } from "@/hooks/useNodeStreamMeta";

const BaseNodeComponent: React.FC<NodeProps<BaseNodeData>> = ({ data, selected }) => {
  const meta = useNodeStreamMeta(data.nodeType);
  // meta.roles = ["producer", "consumer"] 或包含 decision_point / effect_node

  return (
    <div>
      {meta?.roles.includes("decision_point") && <Badge color="blue">{t("stream.role.decision_point")}</Badge>}
      {meta?.roles.includes("effect_node") && <Badge color="orange">{t("stream.role.effect_node")}</Badge>}
      {/* 现有 UI */}
    </div>
  );
};
```

**评估影响**：Sprint 1 阶段**不做此扩展**，避免分心。

---

## 5. 28 节点改造总览矩阵

> 实际为 30 个 executor（含 fallback 内部使用 + llm_resolve helper），下表以 RFC 提及的 28 节点对外可见部分为准。

| # | 节点 | node_type | 业务分类 | RFC Sprint 1 改造 | Sprint 1 行数 |
|---|------|-----------|---------|------------------|-------------|
| 1 | Agent | "agent" | 智能体 | ❌ 自动 | 0 |
| 2 | Aggregator | "aggregator" | 数据聚合 | ❌ 自动 | 0 |
| 3 | Approval | "approval" | 人工 | ❌ 自动（decision_point） | 0 |
| 4 | Code | "code" | 代码 | ❌ 自动 | 0 |
| 5 | Condition | "condition" | 决策 | ❌ 自动（decision_point） | 0 |
| 6 | DataTransformer | "data_transformer" | 数据 | ❌ 自动 | 0 |
| 7 | DatabaseQuery | "db_query" | 数据 | ❌ 自动（effect_node） | 0 |
| 8 | Debate | "debate" | 智能体 | ❌ 自动 | 0 |
| 9 | Delay | "delay" | 控制流 | ❌ 自动 | 0 |
| 10 | DocumentParser | "document_parser" | 数据 | ❌ 自动 | 0 |
| 11 | Email | "email_send" | 副作用 | ❌ 自动（effect_node） | 0 |
| 12 | End | "end" | 控制流 | ❌ 自动 | 0 |
| 13 | FileOperation | "file_operation" | 数据 | ❌ 自动 | 0 |
| 14 | HttpRequest | "http_request" | 副作用 | ❌ 自动（effect_node） | 0 |
| 15 | LLM | "llm" | 智能体 | ❌ 自动 | 0 |
| 16 | LlmClassifier | "llm_classifier" | 决策 | ❌ 自动（decision_point） | 0 |
| 17 | Logging | "logging" | 副作用 | ❌ 自动 | 0 |
| 18 | Loop | "loop" | 控制流 | ❌ 自动 | 0 |
| 19 | Merge | "merge" | 控制流 | ❌ 自动 | 0 |
| 20 | Notification | "notification_send" | 副作用 | ❌ 自动（effect_node） | 0 |
| 21 | Parallel | "parallel" | 控制流 | ❌ 自动 | 0 |
| 22 | SubWorkflow | "sub_workflow" | 控制流 | ❌ 自动 | 0 |
| 23 | Switch | "switch" | 决策 | ❌ 自动（decision_point） | 0 |
| 24 | Tool | "tool" | 副作用 | ❌ 自动 | 0 |
| 25 | Trigger | "trigger" | 控制流 | ❌ 自动 | 0 |
| 26 | Validation | "validation" | 数据 | ❌ 自动 | 0 |
| 27 | VectorRetrieve | "vector_retrieve" | 数据 | ❌ 自动 | 0 |
| 28 | WebhookSend | "webhook_send" | 副作用 | ❌ 自动（effect_node） | 0 |

**总计：28 节点 0 改动**（用 `stream_meta()` 默认实现自动推导 roles）

**唯一核心改造**：
- `node_executor_trait.rs`: +6 行（默认方法）
- `engine.rs::update_node_status`: +10 行（双写）
- `workflow_execution.rs`: +50 行（流日志字段 + 方法）
- `stream_types/*`: +600 行（全新子模块）
- `stream_commands.rs`: +120 行（全新 Tauri 命令）

---

## 6. 迁移路径时间线

```
2026-06-06  评审会（RFC-0001 评审）                 60 分钟
2026-06-07  Sprint 1 启动
2026-06-08  T1.1 - T1.2 完成（stream_types 骨架 + StreamError）       0.5 d
2026-06-09  T1.3 - T1.4 完成（Window/Filter/Aggregate + Event）        1.0 d
2026-06-10  T1.5 - T1.7 完成（Stream trait + Subscription + Factory）  2.0 d
2026-06-11  T1.8 完成（InProcessBus 核心）                            1.5 d
2026-06-12  T1.9 - T1.10 完成（Registry + StreamLogger）              2.0 d
2026-06-13  T1.11 完成（WorkflowExecution 扩展）                      1.0 d
2026-06-14  T1.12 - T1.13 完成（节点元数据 + 锁顺序）                  1.0 d
2026-06-15  T1.14 - T1.15 完成（环检测 + ts-rs）                      1.0 d
2026-06-16  T1.16 - T1.17 完成（Tauri 命令 + i18n）                    1.5 d
2026-06-17  T1.18 - T1.19 完成（性能基线 + 28 节点回归）              1.5 d
2026-06-18  T1.20 完成（质量门禁 + RFC 链接）                         0.5 d
2026-06-19  Sprint 1 Demo + 回顾
2026-06-20  Sprint 1 验收签字 / Sprint 2 启动
```

**Sprint 1 周期**：2 周（10 个工作日）

**风险缓冲**：
- T1.8（InProcessBus 类型擦除）是最复杂任务，预留 1.5 天（含调试时间）
- 如遇类型擦除 Rust 限制，启用"降级方案"（所有 stream 统一为 `serde_json::Value` payload）
- Sprint 1 中如 T1.18 性能不达标，回退到 T1.11 优化 `write_result_and_log` 内联

---

## 7. 评审会前自查清单

**会前 1 天**完成：

- [ ] 已读 [stream-architecture.md §3.1 核心抽象 + §3.4 5 锚点](./stream-architecture.md)
- [ ] 已读 [rfc-0001-stream-runtime.md §3-§6](./rfc-0001-stream-runtime.md)
- [ ] 已读本文 §1 关键架构纠偏
- [ ] 已读本文 §2 LLMNode 改造示例
- [ ] 已读本文 §3 LlmExecutor 改造示例
- [ ] 已读本文 §4 BaseNode.tsx before/after
- [ ] 已 grep 一遍 `node_type` 用法：
  ```bash
  cd src-tauri && rg "fn node_type" -n
  # 应输出 30 个 impl（每个 executor 1 个）
  ```
- [ ] 已查 [node_executor_trait.rs](../../src-tauri/crates/rt-workflow/src/work_engine/node_executor_trait.rs) 确认默认方法位置
- [ ] 已看 [executors/mod.rs](../../src-tauri/crates/rt-workflow/src/work_engine/executors/mod.rs) 确认 30 个 executor 列表
- [ ] 已准备 5 个你最想问的 issue（评审会上提出）

**会前 1 小时**完成：

- [ ] Sprint 1 计划已 grep "TDD" 验证（T1.1-T1.20 全 TDD）
- [ ] 性能基线数字已从现有 28 节点测速准备好（baseline 对比）
- [ ] 准备好说"我反对 XXX，因为 YYY"的具体场景

---

## 8. FAQ（10 个最可能问题）

### Q1：流式执行模型是否会替换现有 DAG 引擎？

**不会**。RFC §1 明确"不替换现有 DAG 引擎，仅通过双写 + 元数据扩展**叠加**新能力"。Sprint 1 的核心约束是"现有 28 节点 e2e 全绿"——任何破坏性改动都过不了准入标准。

### Q2：28 节点元数据改造为什么这么少？

因为：
1. 节点元数据实际载体是 **Rust `NodeExecutorTrait::node_type()`**，不在前端类
2. RFC §3.4 设计 `deriveRolesFromNodeType` **从 node_type 字符串自动推导** roles
3. Sprint 1 **不要求** 28 节点显式声明 `produces/consumes`（Sprint 3 才引入节点级流声明）

所以 30 个 executor **0 改动**，只在 trait 加一个默认方法（+6 行）。

### Q3：双写（results + stream_log）的存储开销？

- 每个节点输出**额外**多一份 stream_log 条目（`WorkflowStreamEvent<Value>`）
- `WorkflowStreamEvent` 结构：id(u64) + topic(String) + timestamp(SystemTime) + payload(Value) + 2 Option<String> = 约 80 字节元数据 + payload
- 假设 100 节点工作流平均 payload 1KB，stream_log 多 ~80KB 内存
- 持久化：Sprint 1 默认 **in-memory + ring buffer（1000 条/topic）**；Sprint 2 后根据需求决定 SQLite BLOB 或独立 log table

### Q4：性能影响到底多大？

- Sprint 1 末必须做的基准（RFC §5.3）：100+ 节点工作流单次执行，**双写开销 P99 < 10%**
- 实现路径：tokio::broadcast 是无锁的，stream_log 写入主要是 JSON 序列化和 HashMap insert
- 风险预案：如果不达标，inline `write_result_and_log` 避免函数调用开销
- 性能测试文件：`src-tauri/benches/stream_dual_write_bench.rs`（Sprint 1 任务 T1.18）

### Q5：跨工作流订阅何时实现？

- Sprint 1：**静态环检测**（DFS，validate_no_cycles）
- Sprint 1：**InProcessBus 默认实现**（进程内，订阅通过 InProcessBus）
- Sprint 3：**跨工作流订阅 first-class**（Tauri 内置事件系统）
- Sprint 4：**可替换后端**（如需 NATS/SSE，可换 trait 实现）

### Q6：ACL / 权限怎么办？

**v1 简化**：工作流级 subscription **不做权限控制**（同一用户的所有工作流可互订）。Sprint 4 引入 ACL。

理由：YAGNI。先验证能力基座跑通，业务权限需求可推迟到 Sprint 4 收到实际反馈后设计。

### Q7：和现有 `streamStore.ts`（ChatStream）冲突吗？

**不冲突**，但需要命名区分（RFC §3.6）：
- `src/stores/domain/streamStore.ts` → ChatStream（chat LLM 流式输出）
- `src/stores/domain/workflowStreamStore.ts` → WorkflowStream（工作流执行流，**新增**）

两个 store 走不同 Tauri 事件通道：
- ChatStream：`onChatChunk` / `emitChatChunk`
- WorkflowStream：`onWorkflowStreamEvent` / `emitWorkflowStreamEvent`

### Q8：types 类型生成用 ts-rs 还是手写？

**ts-rs**（RFC §3.5 + §7 Open Question #5）：
- 优点：单一权威源（Rust 端）、自动同步
- 缺点：build 时长 +10%；首次集成需配置
- 替代方案：specta（更轻量）、tsify（type macro 风格）
- Sprint 1 决策：先用 ts-rs 试水；如 build 慢，再换 specta

### Q9：3 个"业务场景探针"去哪了？

按用户原则（**能力基座而非具体业务**），RFC 把"投资/一人公司/分布式仿真"3 个场景从**设计目标**降级为**覆盖探针**。Sprint 1 demo 不做具体业务，而是：
- 通用"反应式系统模板"：event source → decision → effect（用 LLMNode + Condition + HttpRequest）
- 通用"可观测系统模板"：stream → aggregate → UI panel（用 Aggregator + StreamQueryBuilder）
- Sprint 3 启动时再决定用哪个具体业务作为 e2e 验证

### Q10：评审会通过标准是什么？

参考 [rfc-0001-review-guide.md §5 评审通过标准](./rfc-0001-review-guide.md)：
- 方向锁定：流式执行模型 ✅
- 关键决策点全部有结论（5 个决策点，包括 Stream 元数据载体决策）
- 风险清单有应对方案（双写 / 锁顺序 / 命名冲突 / YAGNI 边界）
- 5 种常见反对意见有预案
- Sprint 1 计划有明确的责任人和时间线

**未通过处理**：列具体修订点，转 RFC-0001-v2，重新走评审。

---

## 附录：评审会现场提问模板

如果你在评审会上想挑战 RFC，可以用以下模板：

```
我担心 [具体场景]：当 [条件] 时，[RFC 设计] 会导致 [具体后果]。
建议 [替代方案] 或者 [缓解措施]。
```

**不好的提问**：
> "这样设计好吗？"

**好的提问**：
> "我担心 100 节点工作流双写：第 50 个节点 stream_log 写入失败时，results 已经写了，会出现"流视图失败但末值视图成功"的不一致。建议在 WorkEngine 双写时把 stream_log 写入移到 results 写入之前，或者用事务包起来。"

---

**文档版本**：v1 · 2026-06-06
**配套使用**：评审会前 1 天发给参会者
**维护者**：RFC-0001 作者
