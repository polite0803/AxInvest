# 供应商与模型

> 状态：active ｜ 日期：2026-11-04 ｜ 对应版本：v3.0.0 ｜ 功能点：provider-models

## 定位

这条链路把「一次模型调用」从 UI 的模型选择落到具体供应商请求：适配器把统一请求翻译成各家协议，流式增量再翻译回统一事件；`tool_calls` 在流式 / 非流式 / 历史序列化三条路径共用一套命名归一；采样参数与 `max_tokens` 走单一覆盖入口；usage 与成本按模型价目表折算；模型类型决定这次调用是否允许生成；多 Key 轮询与 embedding 重试决定失败时是「换一个」还是「立刻失败」。

**边界**：本文件只写供应商适配与模型调用面；会话、工具、审批、压缩、RAG、工作流节点属于 `agent-runtime`。

## 当前行为

### 1. 提供商适配层与注册

适配层是**注册表 + 统一请求结构**：`ProviderRegistry` 持有 `Arc<dyn LlmProvider>`，按 `ProviderType` 找到适配器（`axagent-ai/crates/providers/src/registry.rs:23-44`）。内置适配器由 `register_builtin_providers` 一次注册（`:102`），默认构造与能力注册表注入各有一条路径（`:55-61`、`:124`）。自定义 / OpenAI 兼容端点通过字符串构造走同一个 `OpenAiProvider`，不新增 trait 实现。

适配器只负责协议翻译（请求体、响应解析、流式帧），不做业务决策；模型类型、采样参数优先级、审计由上层统一处理。

（对应版本：v3.0.0（provider 类型安全重构 + 注册表收敛，`CHANGELOG.md:140`）；v2.9.4（供应商列表与能力注册，`CHANGELOG.md:884`））

### 2. 流式输出与 `tool_calls` 命名

`tool_calls` 归一规则集中在 `axagent-ai/crates/providers/src/tool_calls.rs`（模块在 `axagent-ai/crates/providers/src/lib.rs:29` 声明），三条路径共用：流式增量累积、非流式一次性解析、历史消息序列化（`axagent-ai/crates/providers/src/openai.rs:265` 注释明确这一点）。

归一规则：

- `sanitize_tool_name` 修剪空白并把非法字符替换为 `_`，最长 64 字符（`axagent-ai/crates/providers/src/tool_calls.rs:24-31`）；
- `sanitize_messages` 丢弃**空 name** 的 tool call、为空 id 合成稳定 id、剔除**孤儿 tool 消息**（没有对应 assistant tool call 的 `tool` role）（`axagent-ai/crates/providers/src/tool_calls.rs:98-120`）；
- 历史序列化时，assistant 的 `content` 统一为字符串，不传数组（`axagent-ai/crates/providers/src/openai.rs:441-444` 的注释说明这是为兼容中转站）；
- 流式与非流式产出的 `ToolCall` 字段名一致：`id` / `name` / `arguments`（`axagent-ai/crates/providers/src/tool_calls.rs:56-64`）。

非流式请求仍显式带 `stream_options.include_usage`，保证 usage 不因非流式路径缺失（`axagent-ai/crates/providers/src/openai.rs:691-695`）。

（对应版本：v3.0.0（`tool_calls` 三路径统一 + 历史消息空 content 契约，`25db82ebf`））

### 3. 采样参数与 `max_tokens` 失败路径

采样参数的唯一权威是 `model_param_overrides`（`axagent-core/crates/harness/src/types/provider_model.rs:296-301`）：调用方按「节点配置 > 模型覆盖 > 不下发」自己决定优先级，函数只查找、不取舍（`:284-295`）。聊天链路与工作流 LLM 节点都读同一格，避免工作流再走模板变量私通道。

`max_tokens` 字段名按模型判定：o-series / gpt-5 一类必须用 `max_completion_tokens`，其余用 `max_tokens`（`axagent-ai/crates/providers/src/openai.rs:675-686`）。判定是**确定性映射**，没有「发错字段后自动换名重试」：上游若因字段不认返回 400，错误直接冒泡到调用方，由调用方按 `ErrorResponse` 处理。也就是说，手填模型 id 时若判定错，失败点在上游 400，而不是本地参数校验。

请求延时 `request_delay_ms` 同样单点：`request_delay_ms_for_model` 只返回配置值，真正 `sleep` 在 `llm_executor::apply_request_delay` 一处执行；两层各睡一次会得到双倍延时（`axagent-core/crates/harness/src/types/provider_model.rs:272-282`）。

（对应版本：v3.0.0（采样参数单一权威 39b146563；字段名判定与 `tool_calls` 三路径同批 `25db82ebf`））

### 4. Token 计量与成本

usage 统一结构在 `runtime-core`：`input_tokens` / `output_tokens` / `cache_read_input_tokens` / `cache_creation_input_tokens` / `total_tokens` / `total_cost_usd` 等字段一次记录（`axagent-runtime/crates/runtime-core/src/usage.rs:3-18`），记录入口集中在 `record_usage`（`:69-85`）。

成本用价目表折算：`ModelPricing` 的单位是「每 100 万 token 的美元价」，默认表内置主流模型，未知模型落默认档（`axagent-core/crates/harness/src/usage_pricing.rs:22-31`）；Anthropic 的缓存读取 / 写入有独立倍率（`:87-190`）。成本是**估算值**，不替代供应商账单：价目表是静态表，供应商调价后需要改代码。

（对应版本：v3.0.0（usage 字段与成本表统一，`CHANGELOG.md:140`）；历史计量修正见 `CHANGELOG.md:561`、`:567`）

### 5. 模型类型与 Jev 门禁

`resolve_model_type` 是模型类型的唯一解析入口：优先取 provider 登记的 `model_type`，未登记时按模型 id 命名推断（`axagent-core/crates/harness/src/types/provider_model.rs:264-270`）。`is_generation_blocked` 只把 `ModelType::Decision` 判为「不可用于生成」；`Voice` / `Embedding` 各有合法通道，不在该判据范围（`:256-258`、`:254-255`）。

两个消费点必须复用同一函数，不能各自写 `== ModelType::Decision`：聊天发送链路（`src-tauri/src/commands/conversations/streaming.rs:929`、`:946-947`）与工作流 LLM 节点（`axagent-runtime/crates/rt-workflow/src/work_engine/executors/llm_resolve.rs:135`、`:142-144`）。命中阻断时走「换模型 / 报错」的既有降级路径，不会静默把 Decision 模型当生成模型用。

（对应版本：v3.0.0（模型类型唯一权威 11aa58772，`CHANGELOG.md:140`；消费点收口 v2.9.9，`CHANGELOG.md:561`、`:567`、`:588`））

### 6. 多 Key 轮询与限流退避

`get_active_key` 先取该 provider 的**启用** key 列表；单 key 直接返回，多 key 用进程内 `ROUND_ROBIN` 索引轮询（`axagent-infra/crates/dao/src/repo/provider.rs:555-581`）。索引是 `OnceLock<Mutex<HashMap<provider_id, u32>>>`，只在进程内有效，重启后从 0 开始；锁不跨 await，按 `parking_lot` 的 SAFETY 注释使用（`:570-573`）。

失败上报走 `report_key_failure`，只写 `last_error`，**不把 key 移出轮询**（`:583-597`）。因此 401/403/429 之后，同一 provider 的后续请求仍可能轮到坏 key；真正排除坏 key 依赖 `toggle_provider` 的 `last_validated_at` 过滤与人工禁用（`:599-612` 的验证语义）。

首字节等待分软 / 硬两档：软档默认 45s，超时判「慢」→ 同供应商原地重试一次；硬档默认 90s，超时判「挂」→ 上层切供应商；请求体每 64KB 加 5s，软 / 硬封顶 120s / 240s（`axagent-ai/crates/providers/src/stream_budget.rs:33-54`、`:56-63`）。用户可配范围 10–240s（`:44-48`）。

（对应版本：v3.0.0（首字节软 / 硬两档预算，`ff9c46f39`、`CHANGELOG.md:195`）；多 Key 轮询为既有实现，无独立版本提交）

### 7. Embedding 重试与永久错误判定

Embedding 请求走同一 `OpenAiProvider` 的 `/embeddings` 端点：输入单条文本、取 `data[0]`，并校验 `index == 0`，避免某些中转把乱序结果当首条（`axagent-ai/crates/providers/src/openai.rs:1558`、`:1587-1590`）。

建索引链的重试判据是 `permanent_embed_error`：HTTP **4xx 且不是 408 / 429** 判永久错误，立即失败不重试；5xx、408、429 与网络错误才进入指数退避重试（`src-tauri/src/indexing.rs:309-313`、`:425-456`）。这样 401（key 错）/ 404（模型不存在）/ 422（输入非法）不会被无意义地重试 3 次，避免把上游限流打得更狠。

Embedding 模型解析有独立兜底：优先取该 provider 登记的 embedding 模型，未配置时按命名推断；本地 llama.cpp 路径允许「无 key」构造（`src-tauri/src/indexing.rs:677-721`，`CHANGELOG.md:1460`）。

（对应版本：v2.9.9（永久 HTTP 错误快速失败，`CHANGELOG.md:518`）；v2.8.7（embedding_provider 脏数据归一化 + 本地无 key 降级，`CHANGELOG.md:1384`、`:1460`））

## 关键代码入口

- 注册表：
  - `axagent-ai/crates/providers/src/registry.rs:23-44`（`ProviderRegistry`）、`:55-61`（默认构造）、`:102`（内置注册）
  - `axagent-ai/crates/providers/src/lib.rs:29`（`tool_calls` 模块）
- 流式 / `tool_calls`：
  - `axagent-ai/crates/providers/src/tool_calls.rs:24-31`（名称归一）、`:56-64`（序列化）、`axagent-ai/crates/providers/src/tool_calls.rs:98-120`（消息清洗）
  - `axagent-ai/crates/providers/src/openai.rs:265`（三路径共用规则）、`:441-444`（历史 content 契约）、`:691-695`（`include_usage`）
- 参数 / 模型类型：
  - `axagent-core/crates/harness/src/types/provider_model.rs:256-258`（生成阻断）、`:264-270`（类型解析）、`:296-301`（采样覆盖）
  - `src-tauri/src/commands/conversations/streaming.rs:929`、`:946-947`
  - `axagent-runtime/crates/rt-workflow/src/work_engine/executors/llm_resolve.rs:135`、`:142-144`
- 计量 / 成本：
  - `axagent-runtime/crates/runtime-core/src/usage.rs:3-18`、`:69-85`
  - `axagent-core/crates/harness/src/usage_pricing.rs:22-31`、`:87-190`
- Key / 首字节 / embedding：
  - `axagent-infra/crates/dao/src/repo/provider.rs:555-581`、`:583-597`
  - `axagent-ai/crates/providers/src/stream_budget.rs:33-54`
  - `src-tauri/src/indexing.rs:309-313`、`:425-456`、`:677-721`

## 配置与开关

- `max_tokens` 字段名：按模型判定 `max_tokens` 或 `max_completion_tokens`，无运行时开关（`axagent-ai/crates/providers/src/openai.rs:675-686`）。
- 采样覆盖：`ModelParamOverrides`（temperature / top_p / max_tokens / request_delay_ms 等），未配即不下发（`axagent-core/crates/harness/src/types/provider_model.rs:296-301`）。
- 首字节预算：软 45s / 硬 90s（默认），每 64KB +5s，软封顶 120s、硬封顶 240s，用户可配 10–240s（`axagent-ai/crates/providers/src/stream_budget.rs:33-48`）。
- 多 Key：启用列表 + 进程内轮询索引；坏 key 需人工禁用或依赖 `last_validated_at` 过滤（`axagent-infra/crates/dao/src/repo/provider.rs:555-612`）。
- Embedding 重试：4xx（除 408/429）永久失败；其余进入退避（`src-tauri/src/indexing.rs:309-313`）。

## 已知限制与坑

1. **`max_tokens` 字段名判错不会自动纠正**：映射是确定性的，没有 400 后换字段重试；手填 o-series / gpt-5 类模型 id 时要确认 `use_completion_tokens` 判定命中。
2. **多 Key 不剔除失败 key**：`report_key_failure` 只写 `last_error`，轮询索引继续转（`axagent-infra/crates/dao/src/repo/provider.rs:583-597`）；没有「冷却 N 秒」或「连续失败自动禁用」逻辑。要靠 401/403/429 时真正切走，必须在调用方显式禁用或等人工处理。
3. **首字节软档重试会放大成本**：「慢」触发同供应商原地重试一次（`axagent-ai/crates/providers/src/stream_budget.rs:59`）；如果供应商只是慢不是挂，重试会让同一请求发两次，计费与日志都要按「可能双发」理解。
4. **`is_generation_blocked` 只阻断 `Decision`**：`Embedding` / `Voice` 模型不会被它拦住；把 embedding 模型填到生成槽位时，失败点在上游协议，不是本地门禁（`axagent-core/crates/harness/src/types/provider_model.rs:254-258`）。
5. **成本是静态表估算**：供应商调价、阶梯价、缓存价差异都不会自动更新；`total_cost_usd` 只能用于趋势，不可用于对账（`axagent-core/crates/harness/src/usage_pricing.rs:22-31`）。
6. **Embedding 4xx 永久判定不含 408/429**：这两个状态码属于「可重试」；其余 4xx 直接失败。新增供应商若用非标准 4xx 表示限流，会被误判永久错误（`src-tauri/src/indexing.rs:309-313`）。
7. **轮询索引进程内**：多窗口 / 多进程不共享轮询位置；重启后从 0 开始，不能假定「全局均匀」。
8. **`tool_calls` 清洗会丢弃空名调用**：这是有意的兼容策略，但意味着上游若真的产出无名工具调用，本地会静默少一条，而不是报错（`axagent-ai/crates/providers/src/tool_calls.rs:98-120`）。

## 相关文档

| 文档 | 状态 | 版本 | 关联 |
|---|---|---|---|
| `PLAN-llm-param-single-source.md` | 已拍板（2026-10-25，无元信息块） | — | 采样参数 / `max_tokens` 单一权威的上游计划；第 3 节的直接依据 |
| `PLAN-repo-split.md` | active | v3.0.0+（未定版，待确认） | `axagent-ai/crates/providers` 路径迁移；本文件按拆分后目录书写 |
| `PROPOSAL-stream-architecture.md` | APPROVED（导入文档） | — | 工作流能力基座的视角反转方案；流式 / 首字节的上游背景 |
| `PLAN-codex-parity-adoption.md` | active | v2.9.9 | 审批 / Guardian 对齐项；与本文件的模型调用面相邻，不重叠 |

## 版本变更记录

| 版本 | 日期 | 该功能点在本版本改了什么 | 证据（提交短哈希或 CHANGELOG 版本节） |
|---|---|---|---|
| v2.8.7 | 2026-08-02 | `embedding_provider` 历史脏数据归一化 | `CHANGELOG.md:1384` |
| v2.8.7 | 2026-08-02 | llama.cpp 本地推理适配 + embedding 无 key 降级 | `CHANGELOG.md:1460` |
| v2.9.9 | 2026-09-23 | embedding 永久 HTTP 错误快速失败（4xx 除 408/429 不重试） | `CHANGELOG.md:518` |
| v2.9.9 | 2026-09-23 | usage / 成本 / 模型类型消费点收口 | `CHANGELOG.md:561`、`:567`、`:588` |
| v3.0.0 | 2026-09-25 | provider 类型安全重构 + 模型类型唯一权威（`is_generation_blocked` / `resolve_model_type`） | `11aa58772`；`CHANGELOG.md:140` |
| v3.0.0 | 2026-10-26 | 采样参数单一权威（`model_param_overrides`） | `39b146563` |
| v3.0.0 | 2026-10-28 | 首字节软 / 硬两档预算 + 路由 / 顶栏预算统一 | `ff9c46f39`；`CHANGELOG.md:195` |
| v3.0.0 | 2026-10-29 | OpenAI 适配器：流式 / 非流式共用 `tool_calls` 规则、历史消息空 content 契约、`include_usage` | `25db82ebf` |
| v3.0.0 | 2026-10-31 | provider key 解密错误前端分支 i18n 收口 | `acedf88e2` |
| unreleased | 2026-11-01 | 代码按 8 个 workspace 根拆分（`providers` 迁至 `axagent-ai/crates/providers`） | `8dfaeb4db` |
