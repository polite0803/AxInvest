# AUDIT — WorkflowNode 开放化阶段0：位点复核与序列化基线冻结（2026-11-17）

> 状态：active ｜ 日期：2026-11-17 ｜ 对应版本：v3.0.0+（未定版，待确认）
> 关联：`PROPOSAL-workflow-node-open-registry.md` §4 阶段0
> 性质：**时点快照**——记录 2026-11-17 盘上实读的位点行号与字段清单；后续行号漂移不改本文件（快照不订正原则）。

---

## 1. 阶段0 目标与完成度

| 目标（PROPOSAL §4 阶段0） | 状态 | 产物 |
|---|---|---|
| 逐条复核 §2 全部位点的最新行号与符号名 | ✅ 完成 | 本文件 §2 |
| 冻结 32 种节点的序列化样本/字段清单，作为"行为不变"回归基线 | ✅ 完成 | 本文件 §3 |
| 更新 PROPOSAL §2 行号（若有漂移） | ✅ 完成 | 已回写 `PROPOSAL-workflow-node-open-registry.md` §2 |

**未改任何生产代码**——阶段0 纯调研与基线冻结。

## 2. 位点复核（2026-11-17 实读，符号定义处行号）

> 口径：行号取「符号定义所在行」。PROPOSAL §2 原行号多取「match 首臂」，两者差 1–2 行均属同一位点，非漂移事故。

| # | 符号 | 文件 | 实读行号 | 说明 |
|---|---|---|---|---|
| 1 | `pub enum WorkflowNode` | `axagent-core/crates/harness/src/workflow_types.rs` | 1366 | 32 变体 |
| 2 | `pub fn base_id` | 同上 | 1421 | 严格穷举 |
| 3 | `pub fn base(` | 同上 | 1459 | 严格穷举 |
| 4 | `pub fn base_mut` | 同上 | 1497 | 严格穷举 |
| 5 | `pub fn base_output_var` | 同上 | 1558 | 严格穷举 |
| 6 | `fn adjust_position` | `axagent-core/crates/harness/src/assembly_builder.rs` | 385 | 严格穷举 |
| 7 | `fn set_base_id` | 同上 | 426 | 严格穷举 |
| 8 | `pub fn deserialize` | `axagent-core/crates/harness/src/workflow_node_deserializer.rs` | 13（match 在 31） | 32 臂 + 双表 |
| 9 | `unknown_variant` 白名单 | 同上 | 64 | 32 名 |
| 10 | `pub fn node_type_name` | `axagent-runtime/crates/rt-workflow/src/work_engine/node_executor_trait.rs` | 233 | 严格穷举 |
| 11 | `pub fn node_kind` | 同上 | 271 | 严格穷举 |
| 12 | `fn get_node_type` | `axagent-runtime/crates/rt-workflow/src/work_engine/engine/schema_validator.rs` | 104 | 严格穷举 |
| 13 | `pub async fn init_builtin` | `axagent-runtime/crates/rt-workflow/src/work_engine/dispatcher.rs` | 80 | 27 register |
| 14 | 带依赖 executor 构造 | `axagent-runtime/crates/rt-workflow/src/work_engine/engine/mod.rs` | ~1114 / ~1340 | 工厂化对象 |
| 15 | `NODE_TYPE_MAP` | `src/components/workflow/types/workflow.types.ts` | 1005 | 静默降级 |
| 16 | `NodePropertyPanel` | `src/components/workflow/Panels/RightPanel.tsx` | 52 | 32 case + default |
| 17 | PropertyPanels re-export | `src/components/workflow/Panels/PropertyPanels/index.ts` | 3–35 | 32 面板 |
| 18 | `nodeTypes` i18n key | `src/i18n/locales/*.json`（11 语言） | 各 ~9547/9556 | 数据 |

## 3. 序列化基线：32 变体的 type 标签 ↔ wrapper ↔ config 字段

**这是行为不变的判定依据**：阶段1–6 每步改造后，每个变体序列化出的 `{"type": "<标签>", ...base字段, "config": {...}}` 的字段集合必须与下表逐字一致（字段名、serde rename、字段数）。

**关键结构事实**：全部 32 个 wrapper 都是统一的 `{ base: WorkflowNodeBase, config: XxxNodeConfig }` 二元组（无例外）⇒ 阶段1 改为「`node_type` + `#[serde(flatten)] base` + `config: Value`」时，`base` 可无损透传，`config` 保持原 JSON shape。

| 变体 | type 标签 | wrapper(L) | config 类型(L) | config 字段（序列化基线） |
|---|---|---|---|---|
| `Trigger` | `trigger` | `TriggerNode`(L1599) | `TriggerConfig`(L191) | `trigger_type`, `config` |
| `Agent` | `agent` | `AgentNode`(L342) | `AgentNodeConfig`(L267) | `system_prompt`, `context_sources`, `input_mapping`, `output_var`, `model`, `temperature`, `max_tokens`, `tools`, `exposed_tools`, `output_mode`, `agent_profile_id`, `max_tool_rounds`, `execution_mode`, `rag_source_ids`, `consistency_check`, `hallucination_guard`, `fallback_model`, `task_scene`, `stream_chunk_timeout_secs` |
| `Llm` | `llm` | `LLMNode`(L407) | `LLMNodeConfig`(L387) | `model`, `prompt`, `messages`, `temperature`, `max_tokens`, `tools`, `functions`, `consistency_check`, `max_context_tokens`, `reserved_output_tokens` |
| `Condition` | `condition` | `ConditionNode`(L479) | `ConditionNodeConfig`(L459) | `conditions`, `logical_op`, `judge_by_llm`, `routing_prompt`, `routing_model`, `confidence_threshold` |
| `Parallel` | `parallel` | `ParallelNode`(L543) | `ParallelNodeConfig`(L522) | `branches`, `wait_for_all`, `timeout`, `aggregation`, `auto_input_from_parent`, `sub_graph` |
| `Loop` | `loop` | `LoopNode`(L648) | `LoopNodeConfig`(L562) | `loop_type`, `items_var`, `iter_input_var`, `iteratee_var`, `iter_output_var`, `partial_result_var`, `max_iterations`, `continue_condition`, `continue_on_error`, `body_steps`, `interrupt_after_each`, `interrupt_nodes`, `sub_graph` |
| `Merge` | `merge` | `MergeNode`(L686) | `MergeNodeConfig`(L677) | `merge_type`, `inputs`, `auto_inputs_from_branches` |
| `Delay` | `delay` | `DelayNode`(L700) | `DelayNodeConfig`(L693) | `delay_type`, `seconds`, `until` |
| `Validation` | `validation` | `ValidationNode`(L836) | `ValidationNodeConfig`(L817) | `assertions`, `on_fail`, `max_retries` |
| `SubWorkflow` | `subWorkflow` | `SubWorkflowNode`(L757) | `SubWorkflowNodeConfig`(L744) | `sub_workflow_id`, `input_mapping`, `output_var`, `sub_graph` |
| `WorkflowRef` | `workflowRef` | `WorkflowRefNode`(L780) | `WorkflowRefNodeConfig`(L764) | `target_workflow_id`, `input_mapping`, `output_var`, `timeout`, `context_mode` |
| `DocumentParser` | `documentParser` | `DocumentParserNode`(L794) | `DocumentParserNodeConfig`(L787) | `input_var`, `parser_type`, `output_var` |
| `VectorRetrieve` | `vectorRetrieve` | `VectorRetrieveNode`(L810) | `VectorRetrieveNodeConfig`(L801) | `query`, `knowledge_base_id`, `top_k`, `similarity_threshold`, `output_var` |
| `End` | `end` | `EndNode`(L848) | `EndNodeConfig`(L843) | `output_var` |
| `HttpRequest` | `httpRequest` | `HttpRequestNode`(L1017) | `HttpRequestNodeConfig`(L925) | `url`, `method`, `headers`, `body`, `body_type`, `timeout_secs`, `output_var`, `credential_id` |
| `Switch` | `switch` | `SwitchNode`(L895) | `SwitchNodeConfig`(L869) | `input_var`, `cases`, `default_case`, `match_mode`, `use_llm`, `llm_prompt`, `llm_model`, `output_var` |
| `DatabaseQuery` | `databaseQuery` | `DatabaseQueryNode`(L918) | `DatabaseQueryNodeConfig`(L902) | `query`, `params`, `connection_name`, `timeout_secs`, `output_var`, `credential_id` |
| `Notification` | `notification` | `NotificationNode`(L1039) | `NotificationNodeConfig`(L1024) | `channel`, `message`, `webhook_url`, `recipients`, `subject`, `enabled`, `output_var` |
| `Approval` | `approval` | `ApprovalNode`(L1059) | `ApprovalNodeConfig`(L1046) | `message`, `approver`, `timeout_secs`, `timeout_action`, `output_var` |
| `FileOperation` | `fileOperation` | `FileOperationNode`(L1076) | `FileOperationNodeConfig`(L1066) | `operation`, `file_path`, `content`, `output_var` |
| `DataTransformer` | `dataTransformer` | `DataTransformerNode`(L1091) | `DataTransformerNodeConfig`(L1083) | `input_var`, `expression`, `output_var` |
| `WebhookSend` | `webhookSend` | `WebhookSendNode`(L1114) | `WebhookSendNodeConfig`(L1098) | `url`, `method`, `body`, `headers`, `output_var`, `credential_id` |
| `Logging` | `logging` | `LoggingNode`(L1130) | `LoggingNodeConfig`(L1121) | `level`, `message`, `output_var` |
| `LlmClassifier` | `llmClassifier` | `LlmClassifierNode`(L1213) | `LlmClassifierNodeConfig`(L1182) | `categories`, `categories_var`, `prompt`, `model`, `input_var`, `output_var`, `confidence_threshold`, `fallback_label`, `consistency_check` |
| `Aggregator` | `aggregator` | `AggregatorNode`(L1249) | `AggregatorNodeConfig`(L1220) | `strategy`, `input_sources`, `wait_for_all`, `weights`, `summarize_prompt`, `summarize_model`, `output_var`, `sub_graph` |
| `Email` | `email` | `EmailNode`(L1276) | `EmailNodeConfig`(L1256) | `to`, `subject`, `body`, `smtp_host`, `smtp_port`, `smtp_user`, `smtp_pass`, `output_var`, `credential_id` |
| `Debate` | `debate` | `DebateNode`(L1321) | `DebateNodeConfig`(L1283) | `debater_steps`, `max_rounds`, `convergence_prompt`, `convergence_model`, `topic_var`, `output_var`, `sub_graph` |
| `Swarm` | `swarm` | `SwarmNode`(L1357) | `SwarmNodeConfig`(L1332) | `agent_steps`, `max_rounds`, `convergence_prompt`, `convergence_model`, `topic_var`, `output_var`, `sub_graph` |
| `MultiAgent` | `multiAgent` | `MultiAgentNode`(L380) | `MultiAgentNodeConfig`(L353) | `task`, `role`, `model`, `output_var`, `mode`, `max_rounds`, `input_mapping` |
| `Storage` | `storage` | `StorageNode`(L1175) | `StorageNodeConfig`(L1148) | `backend`, `operation`, `input_var`, `collection`, `key_var`, `output_var` |
| `Tool` | `tool` | `ToolNode`(L714) | `ToolNodeConfig`(L707) | `tool_name`, `input_mapping`, `output_var` |
| `Code` | `code` | `CodeNode`(L737) | `CodeNodeConfig`(L721) | `language`, `code`, `output_var`, `tool_name`, `execute_directly`, `input_mapping` |

## 4. 基线使用方法（阶段1–6 执行人必读）

1. **回归比对口径**：改造后对每个变体构造一个最小合法实例，`serde_json::to_value`，断言输出的 `type` 标签、base 字段集合、`config` 字段集合与本表一致；
2. **已有测试缺口**：`axagent-core/crates/harness/tests/serialization_contracts.rs` 目前只覆盖 `WorkflowNodeBase`（:480-510），**未覆盖完整 `WorkflowNode` 32 变体** ⇒ 原计划阶段1补 32 变体往返测试；**阶段1 实况调整**：改为「32 descriptor 类型集合精确断言 + 动态外壳↔enum 往返（storage 样本）+ config 强类型还原」三项（5 个测试全绿，2026-11-17），**32 变体完整往返后移到阶段2**——阶段2 逐个收口 accessor 时天然覆盖每个变体，那时做更省事且不会因必填字段猜测而反复试错（本基线 §3 的字段清单仍是那时的比对依据）；
3. **字段顺序不承诺**：本表字段按源码声明顺序记录，但 serde 输出顺序不构成契约，比对应按字段名集合而非字节序；
4. **快照不订正**：后续代码改动导致行号/字段变化时，**不改本文件**，另起新快照（时点快照原则，见 `AGENTS.md` 文档放置规范）。

## 5. 阶段0 结论

阶段0 完成，可以进入阶段1（引入动态外壳 + descriptor trait，enum 与外壳共存）。进入前需确认：PROPOSAL §7 的三个决策点已由用户裁定（动态外壳模型已批准；阶段4 按"引擎仍是底座"实现；②与 P6.0-② 的先后顺序待定）。
