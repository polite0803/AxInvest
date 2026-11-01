# PLAN — JEV（TypeSafe AI）接入方案

**日期**：2026-09-20
**版本**：v3（§6 四项待裁决**已裁决并落地**；v2 = 接口形态由官方文档确认）
**上游结论**：`AUDIT-jev-adoption-stock-workflow-2026-09-20.md`（§2 适配判定、§4 实施顺序）
**目的**：回答三个问题 —— ① 系统层怎么接；② 用户 UI 怎么设；③ 工作流节点怎么配。
**取证方式**：全部回源码 + 真库（`axinvest`）实测 + 官方文档逐页核对。探针：`output/_probe-jev-{wiring,wiring2,wiring3,nodes,nodes2}.mjs|txt`。

> **v2 相对 v1 的订正**（三处方向性错误，均已回源码/文档证伪）：
> 1. **§0 的「二选一分支」作废** —— 接口形态已确认是**专有 typed API**，不存在兼容壳那条路。
> 2. **❌ v1 写「给 `trader`/`risk-*`/`*-convergence` 上 JEV 做替代」** —— JEV **不生成文本**，这些是产出叙述的 agent 节点 ⇒ 只能做**影子裁判**，不能替代（§4.2）。
> 3. **❌ v1 写「JEV 不需要新节点，扩展 `llmClassifier` 即可」** —— 方向对但**不充分**：`LlmClassifierExecutor` 给 profile 建议供应商传的是 `None`（`:125`），分类器节点**接不到任何 profile 绑定** ⇒ 必须补一条供应商标识通路（§4.4）。

---

## 0. 接口形态（原阻塞项 —— 已解除）

| 要素 | 实测值（文档锚点） |
|---|---|
| 协议 | **专有 typed API** —— `POST https://api.typesafe.ai/v1/systemone`（`/api`） |
| 认证 | `Authorization: Bearer <API_KEY>` |
| 请求体 | `{ state, model, questions }`（`/api#request-body`） |
| `state` | `string \| object \| array`；模型**只认它**，无世界知识、无多轮对话 |
| `questions` | `map<id, Question>`；`Question = { type: "noul"\|"choice"\|"score", instructions, criteria? }` |
| 响应体 | `{ model, answers: map<id, Answer>, usage: {input_tokens, output_tokens} }` |
| 模型 | `jev-1.13.0`（别名 `jev-latest` / `jev-preview`，**均指向 1.13.0**） |
| 模型清单 | `GET https://api.typesafe.ai/v1/models` → `{models:[{name, description, release_date}]}` |
| 官方 SDK | 仅 **Python**（`typesafe-sdk`）与 **JavaScript**（`@typesafe-ai/sdk`）—— **无 Rust SDK** |
| 流式 | **不存在**（文档索引全站无任何 `stream` 页面） |
| 采样参数 | **无** `temperature` / `seed` / `top_p` —— 无法用参数控确定性 |

### 三原语与答案形态（`/api#answer-types`）

| 原语 | 请求 `criteria` 形态 | 答案字段 | 有 `confidence`？ |
|---|---|---|---|
| `noul` | 可选 `{true: "...", false: "..."}` | `noul: 0..1` | **无**（官方明示「Noul answers don't carry one」） |
| `choice` | **必需** `map<option, string\|null>` | `choice` + `probabilities`（和=1）+ `confidence` | ✅ 有 |
| `score` | **必需** array（**≥2 档**） | `score`（可落档间）+ `legend` + `probabilities` + `confidence` | ✅ 有 |

### ★ `confidence` 的真实语义（**与 v1 的隐含假设不同**）

| 事实 | 证据 |
|---|---|
| `confidence` 是**从概率分布派生的独立统计量**，**不等于 `max(probabilities)`** | quickstart 实例：`billing=0.84` 而 `confidence=0.596`；`/api` 实例：`technical=0.85` 而 `confidence=0.82` —— **同为「首选档约 0.85」，confidence 却可差 0.22** |
| 官方**不打算锁定**这个定义 | `/confidence`：「you are never locked into our definition … which is exactly why we give you the full `probabilities`」 |
| 官方给出的起手阈值 | 低置信 **< 0.5** 建议**不要行动**/转人工；高风险动作 **> 0.9** |
| 阈值必须按场景分档 | `/confidence`：「A confidence threshold is not one number」 |

⇒ **对本案的直接后果**：`llmClassifier.confidence_threshold` 现有值（真库 `0.5` / `0.6`）**不可直接沿用**，因为判据从「模型自报的置信度」换成了「JEV 派生的集中度」——**量纲相同、口径不同**。必须按 `#95`（度量口径变更须重标定）在自有数据上重标。

### 限制、计费与错误

| 项 | 值 | 对本案的影响 |
|---|---|---|
| 总上下文 | **64k / 请求**（`state` + 全部问题合计） | `portfolio-mgr` 等 308KB prompt 的先例**绝不能整段喂**（§4.5） |
| 单问约束 | **32k**（`state` + **最长的那一个问题**） | **别把长文本塞进 `instructions`** |
| 计费 | **按输入 token**，输出免费（\$42/Btok） | ⇒ **state 是成本主体** ⇒ **多问题单次调用**才划算（§4.3） |
| 限速 | 250,000 tok/s、1,200 req/min，**官方声明会动态调整** | 与 `429` 退避对齐 |
| 输入类型 | **纯文本**（string / JSON / array of text） | 本仓 workflow 变量已结构化，✅ 契合 |
| 错误码 | `401` 密钥 / `422` 校验失败 / `429` 限速 / `529` 过载 | `429`/`529` 需**指数退避**，并遵守 `retry-after` 头 |
| 数据 | 不用于训练；企业版支持 ZDR | — |

### ⚠ 语言风险（对 A 股链是**硬风险**，v1 未记录）

`/models#language-support` 原文：

> English is the primary training language and where accuracy is currently best. **Other languages, including CJK scripts, are handled but not equally well; test on your own content before relying on Jev for a non-English workload**, and pay close attention to Confidence when routing.

⇒ 本仓 stock-analysis 的 `cls-risk-level` 标签是**中文**（`低风险/中风险/高风险/极高风险`），而 4 个 cognitive 路由的标签是**英文**（`general/devops/finance/...`）。**这直接决定试点排序**（§4.3）。

---

## 1. 系统层怎么接

### 1.1 现状：本仓的「加供应商」惯例，以及**为什么这次不能照抄**

真库 `providers` 表 15 行，除 3 个原生协议外**全是 OpenAI 兼容壳**（`provider_type='openai'` + 自定义 `api_host`）：

| 供应商 | `provider_type` | `api_host` | enabled |
|---|---|---|---|
| Agnes | `openai` | `https://api.agnes-ai.cn/` | **1** |
| CHAT2API | `openai` | `http://127.0.0.1:8080/v1` | **1** |
| NVIDIA | `openai` | `https://integrate.api.nvidia.com/v1` | **1** |
| llama.cpp | `llama_cpp` | `http://localhost:8091` | **1** |
| DeepSeek / 通义 / Kimi / 豆包 / 硅基流动 / GLM / MiniMax | `openai` | 各自 host | 0 |
| OpenAI / OpenAI Responses / Gemini / Claude | 原生 | — | 0 |

**但 JEV 不能走这条路**，两条独立理由：

| # | 理由 | 证据 |
|---|---|---|
| 1 | **JEV 没有 `messages`** —— 只有 `state` + **调用方预先声明的 typed questions**。通用 `chat()` 手里只有 `messages`，**没有任何地方能放 `questions`** | `ChatRequest { messages, model, stream, temperature, … }`（`llm_classifier_executor.rs:153-179`）无对应字段 |
| 2 | 就算硬把问题塞进 prompt，得到的是**「求它吐 JSON 再解析」** —— 而 `cls-risk-level` 在 v55 刚被**从 LLM 判定下沉成 Rhai 确定性代码**，动机正是「LLM 输出类型不可靠 / timeout / `partially_completed`」 | `code_executor.rs:599`（v55 回归注释）、`risk-level.rhai` |

⇒ **结论：必须新增专有 adapter。** 官方无 Rust SDK，只能自研（`reqwest` 直连，结构与 `providers/src/openai.rs` 同族）。

### 1.2 改动点（编译器穷举的部分）

新增 `ProviderType::Jev` 后，**穷尽 `match` 只有 3 处**会被编译器拦下：

| # | 文件 | 位置 | 动作 |
|---|---|---|---|
| 1 | `crates/harness/src/types/provider_model.rs` | `:52-63` | 枚举加变体（`#[serde(rename = "jev")]`） |
| 2 | 同上 | `provider_registry_key()` `:75-84` | 加 `ProviderType::Jev => "jev"`（注释 `:72-74` 明写「新增变体必须同步，否则编译期暴露」） |
| 3 | `src/commands/provider_ctx.rs` | `resolve_provider_adapter` `:11-30` | 加分支（该 match **8 个 arm、无 `_` 兜底** ⇒ 必报错） |
| 4 | `src/commands/llm_wiki.rs` | 第二处同名 match | 加分支 |
| 5 | `crates/providers/src/registry.rs` | `register_all_builtins` `:60-75` | `self.register("jev", Arc::new(JevAdapter::new()))` |
| 6 | `crates/providers/src/jev.rs` | **新文件** | 实现 `ProviderAdapter`（见 §1.3） |
| 7 | 前端供应商类型选项 | §2.1 | 加选项 + **11 语言 i18n** |

**★ 好消息：16 处 `provider_registry_key(&…)` 调用点全部免改。** 该函数是全仓**唯一权威映射**（`provider_model.rs:73` 注明「不允许再定义私有副本」），实测 12 个文件 16 处调用点全部经它解析 ⇒ 加一个 arm 即全链生效。

```rust
// 实测：grep 'provider_registry_key(&' → 12 文件 16 处
//   src/commands/{evaluator,fine_tune,local_model,indexing,plan,research,agency_expert}.rs
//   src/commands/conversations/{mod,streaming,compress}.rs
//   src/commands/workflow_ai/{generate×5,compile,chat}.rs
//   crates/{runtime/harness.rs, runtime/llm_bridge.rs, rt-workflow/.../llm_resolve.rs}
```

> **观察（非本次改动，登记备查）**：`register_all_builtins` 注册了 **13 个 key**，但 `ProviderType` 只有 **8 个变体** —— `deepseek/qwen/glm/kimi/wenxin` 这 5 个 adapter **无法经 `provider_type` 到达**（DB 里这些厂商的 `provider_type='openai'` ⇒ `registry_key="openai"` ⇒ 落到 `OpenAIAdapter`）。`llm_resolve.rs:62-63` 是工作流侧的唯一解析路径，已确认它不读 `builtin_id`。这 5 个注册项是死注册还是另有通路（如 gateway），**未查实，不下结论**。

### 1.3 ★ 能力闸：`chat` 明确拒绝 + 新增带默认实现的 `judge`

`ProviderAdapter`（`harness/src/provider.rs:25-45`）的**必实现面只有 4 个**（无默认实现）：

```rust
async fn chat(&self, ctx, request: Arc<ChatRequest>) -> Result<ChatResponse>;   // 必需
fn chat_stream(&self, ctx, request, cancel_token) -> Stream<...>;               // 必需
async fn list_models(&self, ctx) -> Result<Vec<Model>>;                         // 必需
async fn embed(&self, ctx, request: EmbedRequest) -> Result<EmbedResponse>;     // 必需
// 其余（validate_key / speech / realtime / get_response / delete_response）均有默认实现
```

JEV 的实现策略：

| 方法 | 实现 | 理由 |
|---|---|---|
| `list_models` | `GET /v1/models` 直转 `Model` | 官方接口现成，`validate_key` 默认实现即基于它 ⇒ **免费拿到健康检查** |
| `embed` | 返回 `AxAgentError::Provider("JEV 不提供 embedding")` | 本仓已有 llama.cpp（localhost:8091，bge-m3）承担该职责 |
| `chat` | **返回明确错误**：`"JEV 是 typed 判定模型，不支持对话生成；请在 llmClassifier 节点中使用"` | **能力闸** —— 用户若把 JEV 选成聊天模型，得到的是可读错误而非垃圾输出 |
| `chat_stream` | 同上（返回单项 error 流） | 无流式接口；不做伪流式（伪流式会让上层以为它支持） |
| **`judge`（新增）** | 实现 `POST /v1/systemone` | 见下 |

**新增 `judge` 用「带默认实现」的方式加进 trait** ⇒ **13 个现有 adapter 零改动**：

```rust
/// typed 判定能力。默认不支持；仅 JEV 等 typed 模型覆写。
/// 非 typed 模型必须返回 Err（禁止降级成「prompt 求 JSON」）。
fn supports_typed_judgment(&self) -> bool { false }

async fn judge(&self, _ctx: &ProviderRequestContext, _req: TypedJudgmentRequest)
    -> Result<TypedJudgmentResponse> {
    Err(AxAgentError::Provider("该 provider 不支持 typed 判定".into()))
}
```

`TypedJudgmentRequest` / `Response` 建在 `harness/src/types/`（**纯数据契约，不依赖 providers crate**），逐字段对应 §0 的契约 —— 这样 `providers` 与 `rt-workflow` 只共享 harness 类型，不反向依赖。

### 1.4 DB 落库形态

| 对象 | 填法 |
|---|---|
| `providers` 行 | `provider_type='jev'`、`api_host='https://api.typesafe.ai'`、`api_path='/v1/systemone'`、`builtin_id='typesafe'`、`icon=?`、`enabled=1` |
| `providers` 表实际列名（实测） | `id, name, provider_type, api_host, api_path, enabled, proxy_config, sort_order, created_at, updated_at, custom_headers, icon, builtin_id, tool_adaptation, tool_adaptation_marker_prefix` |
| API Key | 走 `provider_keys`（`axagent_crypto` 加密）；**不进 argv**（纪律 12） |
| `models` 行 | `jev-latest`（别用 `jev-preview` —— 与 latest 同指，官方标注「no preview build available」） |
| 版本 pin | **建议 pin `jev-1.13.0` 而非别名** —— 官方明写别名会漂移（「can change without a change on your side」），且 **JEV 没有 temperature/seed**，`confidence` 标定只能靠 pin 版本维持 |

> ⚠️ `api_path` 的语义要落实：本条路径要**真被 adapter 读取**，否则又是一个「写了没人消费」的字段（#595）。

---

## 2. 用户 UI 怎么设

### 2.1 已有：供应商管理页 + 全局默认模型

| 位置 | 现状 |
|---|---|
| 供应商页 | 15 行现成先例；新增 JEV = 一行 + 类型 `jev` + host + key |
| 全局默认 | `settings` 是 key/value 表（键为 **camelCase**）。真库现值：`defaultProviderId`=`c3ac65b6…`(Agnes)、`defaultModelId`=`agnes-3.0-flash`、`fallback*`=Agnes/`agnes-2.5-flash`、`compression*`=CHAT2API/`deepseek-v4-pro`、`image*`=Agnes/`agnes-image-2.5-flash` |

### 2.2 ⛔ 缺口一：`suggestedProviderId` **没有 UI 控件**（而它是唯一的节点级供应商通路）

| 事实 | 证据 |
|---|---|
| 字段存在 | `crates/entities/src/agent_profiles.rs:22-23` |
| 已接到命令层 | `commands/agent_profile.rs:65`；前端类型 `src/types/agentProfile.ts:25,54` |
| **Profile 编辑器无控件** | `AgentProfileManager.tsx` 内 provider 相关命中**仅 1 处**（`:279` 拷贝时透传） |
| 真库现值 | 39 个 `stock-*` profile **全部 `(null)`**（实测 `count(*)=39`、`count(suggested_provider_id)=0`） |

⇒ **想让某个节点走 JEV，UI 上做不到**（只能改 DB 或调 API）。这是 P1 的必须先修项。

### 2.3 ⛔ 缺口二：「模型角色」下拉是装饰字段（441 个节点带着）

| 事实 | 证据 |
|---|---|
| 面板只有 2 个选项 | `AgentPropertyPanel.tsx:718-722`（`quick_think`/`deep_think`） |
| DB 里是**另一套词表** | 真库实测：清一色 `stock-analyst` / `debater` / `risk-evaluator` / `trader` / `decision-maker` / `explainer` |
| `settings` 无对应键 | 真库全表扫描无 `quickThink`/`deepThink` |
| **Rust 侧零读取** | `\.model_role` 全仓 **0 命中读取点** |

### 2.4 JEV 需要新增的 UI（建议）

| 控件 | 位置 | 作用 |
|---|---|---|
| 供应商类型 `JEV` / `TypeSafe` | 供应商页新建/编辑 | 注册 |
| **模型来源**（JEV 是判定模型，不是聊天模型） | 供应商详情 | 明确告知「仅可用于判定节点」，避免被选成聊天模型后撞 §1.3 的能力闸 |
| **`suggestedProviderId` 选择器** | `AgentProfileManager` | ★ 唯一的节点级供应商通路（§2.2） |
| **影子模式开关** | `llmClassifier` 属性面板 | 只在比对不动驱动（复用现成 `consistency_check`） |
| **阈值（按动作分档）** | 同上 | 依官方建议分档（§0 的 confidence） |
| **版本 pin** | 供应商详情 | 默认 `jev-1.13.0` |

---

## 3. 工作流节点怎么配

### 3.1 ⚠ 先纠正两个直觉

**① 节点不能靠 `model` 字段选供应商。** `resolve_model_for_node`（`crates/dao/src/repo/provider.rs:177-208`）的解析链：

```
effective_provider = session_provider_id .or(profile_suggested_provider)
    ├─ Some → 用该 provider；model = node_model > session_model > 该 provider 首个 enabled 模型
    └─ None → resolve_project_default（settings.default_*）
              model = node_model > session_model > default
```

`node_model` **只当模型 id 透传，从不用于选供应商**。而前端 `ModelSelect` 写的是**复合值** `providerId::modelId`（`ModelSelect.tsx:60`），后端**全仓无 `"::"` 拆分** ⇒ 潜在缺陷；但真库实测**全库 111 个模板、0 个节点 `config.model` 非空**（含 `::` 也是 0）⇒ **从未显形**。（数字经 `_verify-plan-numbers.mjs` 逐项复验；v1 曾写 125，已订正。）（登记，不擅改 —— 先要定语义：允许跨供应商，还是只允许同供应商换模型。）

**② 给节点钉 JEV 的正确姿势是 `agentProfileId → profile.suggested_provider_id`**，而 **`llmClassifier` 这条链是断的**：

```rust
// llm_classifier_executor.rs:119-128 —— 第 6 个参数传的是 None
let (prov, key, model, adapter, api_key) = super::resolve_provider_and_adapter(
    &self.master_key,
    self.provider_registry.as_ref(),
    node_model,
    session_model,
    session_provider_id,
    None,                        // ← profile_suggested_provider：分类器节点不透传
    "LlmClassifierExecutor",
)
```

对比 `agent_executor.rs:490`（agent 节点**有**透传 profile 建议供应商）⇒ **分类器节点是唯一没有该通路的执行器**。后果：`llmClassifier` 想走 JEV，**只能**靠会话级 provider 或**全局默认**（后者会影响所有节点）。⇒ **P1 必须补一条通路**，两种改法：

| 改法 | 内容 | 评价 |
|---|---|---|
| **A** | `LlmClassifierNodeConfig` 加 `provider_id: Option<String>`，`:125` 传 `node.config.provider_id.as_deref()` | **推荐** —— 最小、最直白，节点级自足，不依赖 profile |
| B | 按 `agentProfileId` 解析 profile 的 `suggested_provider_id` 再传入 | 与 agent 节点一致，但要引入 profile 加载（60s TTL 缓存）与一个新的配置面 |

> **⚠️ 2026-09-20 裁决（§6 第 4 项）：A 与 B 都不用。**
> 上表是在「分类器节点只能通过 profile 拿供应商」这一前提下列的；该前提已被推翻 ——
> `llm_classifier_executor.rs:113` **本来就在传** `node_model = c.model`。于是把「节点
> `config.model` 的复合值拆出供应商」做成**统一机制**（§6 第 2 项）后，分类器节点**零后端改动**
> 即获得节点级供应商，且不产生「同一意图两条通道」。A/B 两案留档仅作反面对照。

### 3.2 JEV 在工作流里的**三种**用法（不是「替代节点」一条路）

| 用法 | 形态 | 适用 | 风险 |
|---|---|---|---|
| **① 接管 `llmClassifier`** | `provider_type==Jev` 时走 `judge()` typed 路径，`categories`→`Choice.criteria`、`prompt`→`instructions`、`confidence_threshold`→比 `confidence`、`fallback_label`→降级档 | 纯判定节点（**标签少、可枚举**） | 中（换掉了判定引擎） |
| **② 影子裁判（推荐先做）** | **不动任何节点**，在旁新增一个 JEV 判定节点，消费**与目标节点相同的 state**，输出存到独立变量，只记录不驱动 | 任何有 typed 结论的节点 | **低**（不驱动 ⇒ 不改变行为） |
| **③ 多问题单次调用** | 一个 state 提 N 个问题（Noul/Choice/Score 混用），一次 HTTP 拿全部答案 | 需要对同批数据做多维度判定 | 低（新增节点，不影响存量） |

**③ 的经济性依据**：JEV **按输入 token 计费、输出免费**，而 `state` 是成本主体（≤32k）⇒ **同 state 提 10 个问题的成本 ≈ 1 个问题**。本仓 `llmClassifier` 是「一节点一问题」，若直接拿它跑 10 个维度 = 付 10 次 state 的钱。**所以：单维度用 ①，多维度用 ③，不要用 ① 反复跑。**

### 3.3 ★ 试点落点：**4 个 cognitive 路由优先，不是 stock-analysis**

真库实测**全库只有 6 个 `llmClassifier` 节点**：

| 模板 | 节点 | 分类数 | `confidence_threshold` | `fallback_label` | **标签语言** |
|---|---|---|---|---|---|
| `cognitive_l1_domain_router` | `l1_llm` | 8 | `0.6` | `general` | **英文** |
| `cognitive_l2_cluster_router` | `l2_llm` | **24** | `0.5` | `general` | **英文** |
| `cognitive_l3_capability_router` | `rar_llm_select` | 5 | `0.5` | （未设） | **英文** |
| `cognitive_router_main` | `l1_fallback` | 8 | `0.6` | `general` | **英文** |
| `cognitive_router_main` | `l2_fallback` | 15 | `0.5` | `general` | **英文** |
| `stock-analysis` | `cls-risk-level` | 4 | （未设） | `中风险` | **中文** |

**为什么是它们**：

| 理由 | 依据 |
|---|---|
| **结构同构、零改动** | `categories` → `Choice.criteria`、`prompt` → `instructions`、`confidence_threshold` → `confidence`、`fallback_label` → 降级档 —— 字段一一对应 |
| **规避 JEV 的 CJK 弱项** | 标签是英文（官方明写 CJK「handled but not equally well」） |
| **代价低** | 路由判错 = 走错能力域，可回退；不像投资决策会亏钱 |
| **影子比对机制现成** | `consistency_check`（`workflow_types.rs:1207-1209`）+ `output/rhai-shadow-compare.mjs` 的比对思路可直接复用 |
| **L2 的 24 类是真需求** | 分类多、区分度细 —— 正是 typed `probabilities` 能提供「次优候选」的场景，而纯文本输出给不了 |

> **⚠ 反过来说：`stock-analysis` 的 `cls-risk-level` 不该做 JEV 试点。** 它标签是中文（撞 CJK 风险）、`confidence_threshold` 未设（口径未标定）、且刚被 v55 下沉成 Rhai（把确定性判定换回概率判定 = 方向相反）。

### 3.4 `stock-analysis` 的 JEV 用途（订正 v1）

**23 个 agent 节点全部产出叙述文本**（各 11 个 `analyst` + `bull/bear-r1` + `debate-convergence` + `risk-{agg,con,neu,convergence}` + `trader` + `research-mgr` + `decision-explainer` + `value-investor` + `rule-check` + `quality-fallback`）。JEV **不生成文本** ⇒ **一个都不能替代**。

正确用途 = **影子裁判**（§3.2 用法②），对**与目标节点相同的 state** 提 typed 问题：

| 目标节点 | 现类型 | 可问的 typed 判定 | 原语 | 与什么比对 |
|---|---|---|---|---|
| `risk-convergence` | agent（52.2s） | 「三档分歧程度」 | `score`（有序档） | 它自报的 `disagreement_score` |
| `debate-convergence` | agent（41.2s） | 「多空哪方论证更强」 | `choice{bull,bear,tie}` | 它自报的 `consensus_score` 方向 |
| `trader` | agent（69.0s） | 「该 plan 的动作方向」 | `choice{buy,hold,sell}` | 它输出的 `action` |
| 10 个 `a-*` 分析师 | agent | 「该报告与其引用数据是否自洽」 | `noul`（**一规则一 Noul**） | 无（新增能力，对应官方 citation-check cookbook） |

> `rule-check` 仍**不用 JEV** —— 它是 5 条硬阈值，属「可计算的规则」，官方明示这类必须留在代码里（「Keep known rules, calculations, exact lookups, and execution in code」），且审计已定为下沉 Rhai。

### 3.5 节点字段映射（`llmClassifier` 走 JEV typed 分支时）

| 节点字段 | JEV 侧 | 备注 |
|---|---|---|
| `input_var` → 值 | `state` | 传**结构化变量**而非长文本（32k 约束） |
| `prompt`（渲染后） | `choice.instructions` | **注意别超长** —— 它与 state 共享 32k 预算 |
| `categories: Vec<String>` | `choice.criteria` 的**键** | 值为 `null`（官方允许「use null when an option needs no extra detail」） |
| （新增）`criteria_desc` | `choice.criteria` 的**值** | 可显著提升判别力（官方 rubric 指导） |
| `confidence_threshold` | 与响应 `confidence` 比 | ⚠ **口径变了 ⇒ 必须重标定**（§0） |
| `fallback_label` | 低置信 / 429 / 529 / 超时时输出 | **必须保留** —— v9 补丁已实证：节点 Failed 会经 Direct 边阻塞下游（`cls-risk-level → portfolio-mgr` 死锁整条决策链） |

**若要用 `score`**（`risk-convergence` 那类「有序档 + 落档间」）：不要新建节点类型。`ClassificationNodeConfig` 加一个 `question_kind: Option<"choice"\|"score">` 即可（`score` 要求 `categories` 长度 ≥2 且有序）。

---

## 4. 落地顺序

| 阶段 | 动作 | 依赖 | 风险 |
|---|---|---|---|
| **P0** | 契约类型（`TypedJudgmentRequest/Response`）+ `JevAdapter`（4 必需方法 + `judge`）+ 注册 + 3 处穷举 match + `providers` 行 | 无 | 低（纯新增，13 个存量 adapter 零改动） |
| **P1** | **补 UI 缺口**：`suggestedProviderId` 选择器（§2.2）；`LlmClassifierNodeConfig` 加 `provider_id`（§3.1 改法 A） | P0 | 低 |

> **P1 现状（2026-09-20）**：`suggestedProviderId` 选择器 **仍未做**（缺口一原样存在 —— 真库 39 个
> `stock-*` profile 依旧全 `(null)`）。而原计划的「给 `LlmClassifierNodeConfig` 加 `provider_id`」
> 已**经裁决作废**：改由统一的 `config.model` 复合值机制覆盖（§6 第 2/4 项），分类器面板的模型/
> 阈值/降级档控件**已补齐**。⇒ P1 剩余项**只有** `suggestedProviderId` 选择器一件。
| **P2** | **影子模式**：拿 `cognitive_l2_cluster_router.l2_llm`（24 类、英文标签）与 JEV 并跑，只比对不驱动 | P1 | 低（不驱动） |
| **P3** | 依 P2 数据**重标 `confidence_threshold`**（自有数据，分档），再放开驱动 | P2 | 中 |
| **P4** | （可选）多问题节点（§3.2 用法③），用于同 state 多维度判定 | P3 | 中 |

**P2 的验收判据**（沿用本仓既有影子比对口径，见 `rhai-shadow-compare.mjs` 的先例：档位一致率 64.4%、posterior 位移 +0.15pp）：一致率、位移方向、以及**低置信占比**必须三项同时给出 —— 只看一致率会掩盖「JEV 在长 state 上整体退化」（官方有 `model-jaggedness` 专页说明精度随 state 增长而劣化）。

---

## 5. 风险登记

| # | 风险 | 强度 | 应对 |
|---|---|---|---|
| J1 | **CJK 精度**（官方自陈非最佳） | **高** | 试点选英文标签面；中文面须先在自有语料上测 |
| J2 | **`confidence` 口径不可沿用** | **高** | P3 重标定；不重标就投产 = 沿用错口径 |
| J3 | **无流式 / 无采样参数** | 中 | `chat`/`chat_stream` 明确拒绝；pin 版本 ID 保标定稳定 |
| J4 | 无 SLA + 限速动态调整 | 中 | 保留 `fallback_label`（防死锁，v9 已实证）；429/529 指数退避 + `retry-after` |
| J5 | 32k（state + 最长单问）易撞 | 中 | 只喂结构化 state；**不整段喂 308KB 级 prompt** |
| J6 | 别名漂移导致标定失效 | 中 | pin `jev-1.13.0` |
| J7 | 单问题调用浪费 state 成本 | 中 | 多维度走 §3.2 用法③ |
| J8 | 误把 JEV 选为聊天模型 | 低 | §1.3 能力闸（返回可读错误） |

> ⛔ **不得接入**：`crates/prompt-guard/**`（零 LLM 纯正则 4 层）—— 把安全链依赖到公网 API 不可接受（审计 §2.3）。

---

## 6. 待裁决 → **已裁决并落地**（2026-09-20）

四项均给出**建议值**并已实施。每项建议值都附判据与取证锚点 —— 是判断，不是偏好。

| # | 项 | **裁决值** | 判据 | 落地 |
|---|---|---|---|---|
| 1 | `modelRole`（441 节点带着） | **删**（不是接活） | ① 读取端**零命中** —— 全仓唯一命中是 `workflow_types.rs` 的 serde alias，只把值反序列化进 `AgentNodeConfig.model_role` 字段，而**该字段本身无任何消费点**；② UI 词表（`quick_think`/`deep_think`）与 DB 词表（`stock-analyst`/`debater`/`risk-evaluator`/…）**互不相交** ⇒ 即便接活，441 个存量值也全部无法映射；③ 角色的正式载体是 `agent_profile_id`（字段注释明写「唯一标识角色的方式，不再使用旧 role/agent_role_override」）⇒ `model_role` 属**被取代的旧机制残留**；④ 两条**谎话注释**坐实「看起来能配」的幻觉 —— `seed_serenity.rs` 的「确保 domain_constraints 按 stock-analyst 角色注入」（该文件 `domain_constraints` 零命中）、`tools/src/tools/skill/prompt_cache.rs` 的「根据 model_role 自动加载对应 skill」（该文件无任何 `model_role` 消费） | ✅ 字段 + 44 处写入端 + 前端类型/控件 + 11×7 i18n key 全删；DB 存量**不迁移**（无 `deny_unknown_fields`，未知键被 serde 忽略） |
| 2 | 节点 `config.model` 复合串语义 | **允许跨供应商**（复合值 = provider + model **都要**） | ① 仓内两个同形先例已统一为此语义：`src/indexing.rs::parse_embedding_provider`、前端 `DefaultModelSettings.tsx` 把 `ModelSelect` 的值**拆成两个键**再落库；② 前端 `ModelSelect` 的展示是「模型名 (供应商名)」⇒ 若后端只取 modelId，选「同供应商换模型」会让 **UI 撒谎**；③ 反向选择（限同供应商）还需新增一致性校验，成本更高、语义更绕 | ✅ `dao/src/repo/provider.rs` 新增 `split_node_model()`；`resolve_model_for_node` 内节点级 provider 优先级最高；无效 provider **不再静默**（`tracing::warn!`）后降级 |
| 3 | `fallback_label` 低置信/挂掉时的取向 | **按代价方向分档**：路由类（可逆）→ 中性档维持；风险类（不可逆）→ **保守档** | ① **`degraded` 对决策路径不可见** —— `portfolio-mgr.rhai` 的 f4 因子只读 `overall_risk_llm`（= `category`），switch 里中 `-0.05` / 高 `-0.25`，**不吃 `degraded`** ⇒ 中性兜底在决策链上等价于「我判定它是中风险」，把「没判定」伪装成「已判定」；② 官方 JEV 指导同向：「低置信 < 0.5 建议**不要行动**」；③ **存量影响面 = 0**：真库 59 条可比对样本里 `degraded` 零命中（`output/rhai-shadow-report.txt` 中「LLM 档缺失(f4=0.0)」的 4 条其 vol 均存在）⇒ 纯防御性改动 | ✅ `risk-level.rhai` 的 `degraded` 分支「中风险」→「高风险」；`TEMPLATE_VERSION` 59→**60** |
| 4 | `llmClassifier` 供应商通路 A/B | **不新增字段**，走 ② 的统一机制 | `llm_classifier_executor.rs:113` **已经在传** `node_model = c.model` ⇒ ② 落地后分类器**自动**获得节点级供应商。若另加 `provider_id` 字段，则「同一意图（节点指定供应商）」存在两条通道 —— 正是本仓判据反对的双通道 | ✅ 零后端改动（由 ② 覆盖）；补 `LlmClassifierPropertyPanel` 的模型/阈值/降级档三控件 + 修该面板原有 i18n 硬编码 |

> **⛔ 第 3 项只改 `degraded` 一支，别读成「改了一半」。**
> `if level == ""` 有两个**来源不同**的入口：① `degraded`（数据缺失 ⇒ **未知**）；② 规则全不命中
> （数据齐全 ⇒ **真判定**，prompt 明写「A股大多数股票应落在中风险档」）。前者要保守，后者本就该
> 是中风险。合并处理会把真判定一起推高，是**语义降级**而非保守化。

### 6.1 同源但不在裁决范围、一并处理的

| 项 | 处理 |
|---|---|
| `DebateNodeConfig.convergence_model_role` | **同型装饰字段**（Rust 侧零读取、面板有控件、词表同为 `quick_think`/`deep_think`）⇒ 与第 1 项一并删除 |

### 6.2 保持登记、未改动的

| 项 | 状态 |
|---|---|
| 节点 provider 解析**失败**时是否应 fail-stop（而非 WARN + 降级） | **保持 WARN + 降级** —— 与 `session_provider_id` 的既有行为一致（不引入新的失败模式）；且 `llm_resolve.rs` 的 `[LLM-RESOLVE] 选中 provider` info 日志已让「实际跑在谁身上」可审计 |

---

## 7. 复现与锚点

```
node output/_probe-jev-wiring.mjs     # 供应商 / settings / 节点字段（v1，两处口径错误已在脚本头记明）
node output/_probe-jev-wiring2.mjs    # 修正版：camelCase 键 + models 表名 + llmClassifier 字段
node output/_probe-jev-wiring3.mjs    # 全库：config.model 0 命中 / modelRole 441 / 模板清单
node output/_probe-jev-nodes.mjs      # stock-analysis 节点全清单 + 容器 config 键 + providers
node output/_probe-jev-nodes2.mjs     # providers 列名 + 6 个 llmClassifier 的 categories/threshold 实测
```

**文档锚点**（官方，引用前请复验 —— 判据 #591）：

| 主题 | URL |
|---|---|
| Quickstart | `https://docs.typesafe.ai/introduction/quickstart` |
| API 参考（请求/响应/错误） | `https://docs.typesafe.ai/api` |
| confidence 语义 | `https://docs.typesafe.ai/confidence` |
| 模型 / 限制 / 语言支持 | `https://docs.typesafe.ai/models` |
| 索引（`llms.txt`） | `https://docs.typesafe.ai/llms.txt` |

**源码锚点**（引用前请复验 —— 判据 #591）：

| 主题 | 位置 |
|---|---|
| adapter 接口 + 默认实现 | `crates/harness/src/provider.rs:25-120` |
| ProviderType + registry key | `crates/harness/src/types/provider_model.rs:52-84` |
| 注册表 13 个内置 adapter | `crates/providers/src/registry.rs:53-76` |
| 穷举 match 之一 | `src/commands/provider_ctx.rs:11-30` |
| 工作流侧唯一解析路径 | `crates/rt-workflow/.../executors/llm_resolve.rs:62-68` |
| 分类器执行器（含 `None` 处） | `crates/rt-workflow/.../executors/llm_classifier_executor.rs:119-128` |
| 分类器节点契约 | `crates/harness/src/workflow_types.rs:1182-1217` |
| 模型解析链 | `crates/dao/src/repo/provider.rs:177-208` |
| node→profile 通路（agent 侧） | `agent_executor.rs:439,490` |
| 节点模型 UI / profile UI | `AgentPropertyPanel.tsx:690-726`、`settings/AgentProfileManager.tsx:279` |
| 成本主体（308KB prompt 先例） | `crates/providers/src/openai.rs:987-996` |
