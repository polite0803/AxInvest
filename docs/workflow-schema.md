# 工作流 Schema 文档

> 自动生成自 `axagent-harness::workflow_types`。

## 核心类型

| 类型 | 说明 |
|------|------|
| `WorkflowNode` | 工作流节点（28 种变体的标签联合）|
| `WorkflowEdge` | 工作流边定义 |
| `WorkflowTemplateInput` | 工作流模板创建/更新入参 |
| `WorkflowTemplateResponse` | 工作流模板查询响应 |
| `TriggerConfig` | 触发配置（manual/schedule/webhook/event）|
| `RetryConfig` | 节点重试策略 |
| `ErrorConfig` | 节点错误处理配置 |

## WorkflowNode 变体

| `type` 标签 | 节点类型 | 配置结构体 |
|------------|----------|------------|
| `trigger` | `TriggerNode` | `TriggerConfig` |
| `agent` | `AgentNode` | `AgentNodeConfig` |
| `llm` | `LLMNode` | `LLMNodeConfig` |
| `condition` | `ConditionNode` | `ConditionNodeConfig` |
| `parallel` | `ParallelNode` | `ParallelNodeConfig` |
| `loop` | `LoopNode` | `LoopNodeConfig` |
| `merge` | `MergeNode` | `MergeNodeConfig` |
| `delay` | `DelayNode` | `DelayNodeConfig` |
| `tool` | `ToolNode` | `ToolNodeConfig` |
| `code` | `CodeNode` | `CodeNodeConfig` |
| `subWorkflow` | `SubWorkflowNode` | `SubWorkflowNodeConfig` |
| `workflowRef` | `WorkflowRefNode` | `WorkflowRefNodeConfig` |
| `end` | `EndNode` | `EndNodeConfig` |
| `switch` | `SwitchNode` | `SwitchNodeConfig` |
| `httpRequest` | `HttpRequestNode` | `HttpRequestNodeConfig` |
| `databaseQuery` | `DatabaseQueryNode` | `DatabaseQueryNodeConfig` |
| `notification` | `NotificationNode` | `NotificationNodeConfig` |
| `approval` | `ApprovalNode` | `ApprovalNodeConfig` |
| `fileOperation` | `FileOperationNode` | `FileOperationNodeConfig` |
| `dataTransformer` | `DataTransformerNode` | `DataTransformerNodeConfig` |
| `webhookSend` | `WebhookSendNode` | `WebhookSendNodeConfig` |
| `logging` | `LoggingNode` | `LoggingNodeConfig` |
| `llmClassifier` | `LlmClassifierNode` | `LlmClassifierNodeConfig` |
| `aggregator` | `AggregatorNode` | `AggregatorNodeConfig` |
| `email` | `EmailNode` | `EmailNodeConfig` |
| `debate` | `DebateNode` | `DebateNodeConfig` |
| `validation` | `ValidationNode` | `ValidationNodeConfig` |
| `documentParser` | `DocumentParserNode` | `DocumentParserNodeConfig` |
| `vectorRetrieve` | `VectorRetrieveNode` | `VectorRetrieveNodeConfig` |

## 关键字段说明

（本 schema 无共享 definitions）

## EdgeType 枚举

| 值 | 说明 |
|----|------|
| `direct` | 直接连线（默认） |
| `conditionTrue` | 条件节点 true 分支 |
| `conditionFalse` | 条件节点 false 分支 |
| `loopBack` | 循环回边 |
| `parallelBranch` | 并行分支边 |
| `merge` | 合并边 |
| `debateRound` | 辩论轮次边 |
| `error` | 错误处理边 |
| `grouping` | 装饰分组边（不参与校验和布局） |

## 嵌套限制

- WorkflowRef 嵌套深度：≤ 3 层
- 循环引用检测：执行时回溯调用栈

## 向后兼容

- 所有 Optional 字段均有 `#[serde(default)]`，向前兼容
- `kind` 字段默认 `"executable"`，旧数据自动兼容
- `edge_type` 新增 `"grouping"`，旧解析器忽略未知值

---
*文档自动生成，更新类型后请重新运行 `cargo run -p schema-gen`*
