# 工作流 Schema 文档

> 状态：active ｜ 日期：2026-10-30 ｜ 对应版本：v2.9.9


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

## 附录：上游 AxAgent 节点配置字段明细（v2.9.7 时代，2026-09-07）

> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04
> 说明：以下逐类型字段明细来自上游同文件；本仓正文保留 EdgeType / 嵌套限制 / 向后兼容三节（上游无）。

### `AgentNodeConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `agent_profile_id` | `object` | ❌ | AgentProfile ID — 唯一标识角色的方式，不再使用旧 role/agent_role_override |
| `consistency_check` | `object` | ❌ | 结果一致性检查配置（可选，不配置时零影响） |
| `context_sources` | `array` | ✅ |  |
| `execution_mode` | `object` | ❌ | 执行模式: "react" = 逐步思考-行动（默认）, "plan" = 先规划为工作流再执行 |
| `exposed_tools` | `array` | ❌ | 暴露给 LLM 的工具名列表（tools 的子集）。为空时暴露全部（向后兼容）。 固定工具（上游 ToolNode 结果已通过 context_sources 注入）不应暴露。 |
| `hallucination_guard` | `object` | ❌ | 防幻觉锚定检查配置（可选，不配置时零影响） |
| `input_mapping` | `object` | ❌ | 输入变量映射：将工作流变量（如 trigger 输出）注入到 Agent 的 system_prompt 中。 key = 注入到 prompt 的变量名，value = ExecutionState.variables 中的键。 运行时自动解析并追加 `【key】:value` 格式到 system_prompt 尾部。 示例: `{"stock_code": "trigger", "stock_name": "trigger"}` → 注入 "【stock_code】:600036\n【stock_name】:招商银行" |
| `max_tokens` | `object` | ❌ |  |
| `max_tool_rounds` | `object` | ❌ | Agent 多轮工具调用最大轮数，默认 1（不配置则仅单轮） |
| `model` | `object` | ❌ |  |
| `model_role` | `object` | ❌ |  |
| `output_mode` | `object` | ✅ |  |
| `output_var` | `string` | ✅ |  |
| `rag_source_ids` | `array` | ❌ | RAG 知识源 ID 列表。格式: "knowledge:<kb_id>", "memory:<ns_id>", "wiki:<wiki_id>"。 执行时从这些源检索与 query 相关的内容注入 system prompt。 |
| `system_prompt` | `string` | ✅ |  |
| `temperature` | `object` | ❌ |  |
| `tools` | `array` | ✅ | 工具列表，支持向后兼容旧格式 `["name1", "name2"]` |

### `AggregatorNodeConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `input_sources` | `array` | ❌ |  |
| `output_var` | `string` | ❌ |  |
| `strategy` | `string` | ❌ |  |

### `ApprovalNodeConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `approver` | `object` | ❌ |  |
| `message` | `string` | ✅ |  |
| `output_var` | `string` | ❌ |  |
| `timeout_action` | `string` | ❌ |  |
| `timeout_secs` | `integer` | ❌ |  |

### `CodeNodeConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `code` | `string` | ✅ |  |
| `language` | `string` | ✅ |  |
| `output_var` | `string` | ✅ |  |
| `tool_name` | `object` | ❌ | Rhai 脚本注册为工具名（language="rhai" 时生效，为空则用 code_<node_id>） |

### `CompensationConfig`

补偿配置：定义节点失败时的恢复策略

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `compensation_nodes` | `array` | ❌ | 需要执行补偿的节点 ID 列表（预留扩展，当前由引擎根据 DAG 自动推导下游） |
| `strategy` | `object` | ✅ |  |

### `CompensationStrategy`

补偿策略：当节点失败时，如何处理其下游节点和输出


### `ConditionNodeConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `conditions` | `array` | ✅ |  |
| `confidence_threshold` | `object` | ❌ | 置信度阈值（0.0 - 1.0）。LLM 路由返回的置信度低于此值时， 降级为启发式判断（已有的 fallback 逻辑）。 None = 不检查置信度（向后兼容）。 |
| `judge_by_llm` | `object` | ❌ | 启用 LLM 动态路由：由 AI 判断走哪条分支（忽略 conditions 静态规则） |
| `logical_op` | `object` | ✅ |  |
| `routing_model` | `object` | ❌ | LLM 路由使用模型（为空则用系统默认） |
| `routing_prompt` | `object` | ❌ | LLM 路由时的提示词（描述路由判断逻辑） |

### `ConsistencyCheckConfig`

一致性检查配置

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `deviation_threshold` | `number` | ✅ | 结果偏差阈值（0.0-1.0），超过则告警 |
| `enabled` | `boolean` | ✅ | 是否启用 |
| `mode` | `object` | ✅ | 检查模式：相同模型重复执行 / 不同模型交叉验证 |
| `secondary_model` | `object` | ❌ | 可选：用于交叉验证的备用模型名 |

### `DataTransformerNodeConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `expression` | `string` | ✅ |  |
| `input_var` | `string` | ✅ |  |
| `output_var` | `string` | ❌ |  |

### `DatabaseQueryNodeConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `connection_name` | `object` | ❌ |  |
| `output_var` | `string` | ❌ |  |
| `params` | `array` | ❌ |  |
| `query` | `string` | ✅ |  |
| `timeout_secs` | `integer` | ❌ |  |

### `DebateNodeConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `convergence_model` | `object` | ❌ |  |
| `convergence_model_role` | `object` | ❌ |  |
| `convergence_prompt` | `object` | ❌ |  |
| `debater_steps` | `array` | ❌ |  |
| `max_rounds` | `integer` | ❌ |  |
| `output_var` | `string` | ❌ |  |
| `topic_var` | `string` | ❌ |  |

### `DelayNodeConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `delay_type` | `string` | ✅ |  |
| `seconds` | `integer` | ✅ |  |
| `until` | `object` | ❌ |  |

### `DocumentParserNodeConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `input_var` | `string` | ✅ |  |
| `output_var` | `string` | ✅ |  |
| `parser_type` | `string` | ✅ |  |

### `EmailNodeConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `body` | `string` | ✅ |  |
| `output_var` | `string` | ❌ |  |
| `smtp_host` | `object` | ❌ |  |
| `smtp_pass` | `object` | ❌ |  |
| `smtp_port` | `object` | ❌ |  |
| `smtp_user` | `object` | ❌ |  |
| `subject` | `string` | ✅ |  |
| `to` | `array` | ✅ |  |

### `EndNodeConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `output_var` | `object` | ❌ |  |

### `ErrorConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `compensation_steps` | `object` | ❌ |  |
| `error_branch` | `object` | ❌ |  |
| `on_failure` | `object` | ✅ |  |
| `retry_policy` | `object` | ❌ |  |

### `FileOperationNodeConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `content` | `object` | ❌ |  |
| `file_path` | `string` | ✅ |  |
| `operation` | `string` | ✅ |  |
| `output_var` | `string` | ❌ |  |

### `HallucinationGuardConfig`

防幻觉锚定配置

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `enabled` | `boolean` | ✅ |  |
| `match_threshold` | `number` | ✅ | 引用匹配阈值（0-1），低于此值判定为幻觉 |

### `HttpRequestNodeConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `body` | `object` | ❌ |  |
| `body_type` | `string` | ❌ |  |
| `headers` | `object` | ❌ |  |
| `method` | `string` | ❌ |  |
| `output_var` | `string` | ❌ |  |
| `timeout_secs` | `integer` | ❌ |  |
| `url` | `string` | ✅ |  |

### `LLMNodeConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `consistency_check` | `object` | ❌ | 结果一致性检查配置（可选，不配置时零影响） |
| `functions` | `object` | ❌ |  |
| `max_context_tokens` | `object` | ❌ | 最大上下文 token 数（可选，默认 128000） |
| `max_tokens` | `object` | ❌ |  |
| `messages` | `object` | ❌ |  |
| `model` | `string` | ✅ |  |
| `prompt` | `string` | ✅ |  |
| `reserved_output_tokens` | `object` | ❌ | 为输出保留的 token 数（可选，默认 4000） |
| `temperature` | `object` | ❌ |  |
| `tools` | `object` | ❌ |  |

### `LlmClassifierNodeConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `categories` | `array` | ✅ |  |
| `confidence_threshold` | `object` | ❌ | 置信度阈值（0.0 - 1.0）。LLM 返回的置信度低于此值时， 使用 fallback_label（如果配置）或标记为 low_confidence 并返回错误。 None = 不检查置信度（向后兼容）。 |
| `consistency_check` | `object` | ❌ | 结果一致性检查配置（可选，不配置时零影响） |
| `fallback_label` | `object` | ❌ | 置信度不足时的降级标签（可选）。不配置时直接标记失败。 |
| `input_var` | `string` | ✅ |  |
| `model` | `object` | ❌ |  |
| `output_var` | `string` | ❌ |  |
| `prompt` | `string` | ✅ |  |

### `LoggingNodeConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `level` | `string` | ❌ |  |
| `message` | `string` | ✅ |  |
| `output_var` | `string` | ❌ |  |

### `LoopNodeConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `body_steps` | `array` | ✅ |  |
| `continue_condition` | `object` | ❌ |  |
| `continue_on_error` | `boolean` | ✅ |  |
| `items_var` | `object` | ❌ |  |
| `iteratee_var` | `object` | ❌ |  |
| `loop_type` | `object` | ✅ |  |
| `max_iterations` | `object` | ❌ |  |

### `MergeNodeConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `auto_inputs_from_branches` | `boolean` | ❌ |  |
| `inputs` | `array` | ✅ |  |
| `merge_type` | `object` | ❌ |  |

### `NotificationNodeConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `channel` | `string` | ✅ |  |
| `enabled` | `boolean` | ✅ |  |
| `message` | `string` | ✅ |  |
| `output_var` | `string` | ❌ |  |
| `recipients` | `array` | ❌ |  |
| `subject` | `object` | ❌ |  |
| `webhook_url` | `object` | ❌ |  |

### `ParallelNodeConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `aggregation` | `object` | ❌ |  |
| `auto_input_from_parent` | `boolean` | ❌ |  |
| `branches` | `array` | ✅ |  |
| `timeout` | `object` | ❌ |  |
| `wait_for_all` | `boolean` | ✅ |  |

### `RetryConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `backoff_type` | `object` | ✅ |  |
| `base_delay_ms` | `integer` | ✅ |  |
| `enabled` | `boolean` | ✅ |  |
| `max_delay_ms` | `integer` | ✅ |  |
| `max_retries` | `integer` | ✅ |  |

### `RhaiToolDef`

Rhai 脚本工具定义（不属于 DAG 节点，仅作为工具注册）

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `code` | `string` | ✅ | Rhai 脚本代码 |
| `description` | `object` | ❌ | 工具描述（发给 LLM） |
| `tool_name` | `string` | ✅ | 注册为工具名（Agent exposed_tools 引用此名） |

### `SubWorkflowNodeConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `input_mapping` | `object` | ✅ |  |
| `is_async` | `boolean` | ✅ |  |
| `output_var` | `string` | ✅ |  |
| `sub_workflow_id` | `string` | ✅ |  |

### `SwitchNodeConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `cases` | `array` | ✅ |  |
| `default_case` | `object` | ❌ |  |
| `input_var` | `string` | ✅ |  |
| `match_mode` | `string` | ❌ |  |
| `output_var` | `string` | ❌ |  |

### `ToolDef`

工具定义 —— 包含名称、描述和参数 JSON Schema。

反序列化支持向后兼容：旧格式的纯字符串自动转为 ToolDef { name, ..Default::default() }。

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `description` | `object` | ❌ |  |
| `name` | `string` | ✅ |  |
| `parameters` | `object` | ❌ |  |

### `ToolNodeConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `input_mapping` | `object` | ✅ |  |
| `output_var` | `string` | ✅ |  |
| `tool_name` | `string` | ✅ |  |

### `TriggerConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `config` | `object` | ✅ |  |
| `type` | `object` | ✅ |  |

### `ValidationNodeConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `assertions` | `array` | ✅ |  |
| `max_retries` | `integer` | ✅ |  |
| `on_fail` | `string` | ✅ |  |

### `VectorRetrieveNodeConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `knowledge_base_id` | `string` | ✅ |  |
| `output_var` | `string` | ✅ |  |
| `query` | `string` | ✅ |  |
| `similarity_threshold` | `object` | ❌ |  |
| `top_k` | `integer` | ✅ |  |

### `WebhookSendNodeConfig`

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `body` | `object` | ❌ |  |
| `headers` | `object` | ❌ |  |
| `method` | `string` | ❌ |  |
| `output_var` | `string` | ❌ |  |
| `url` | `string` | ✅ |  |

---
*文档自动生成，更新类型后请重新运行 `cargo run -p schema-gen`*
