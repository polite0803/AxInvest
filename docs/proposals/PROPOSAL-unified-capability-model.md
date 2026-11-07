> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# 统一能力建模（Unified Capability Model）

> 状态：**已评审，Phase 0/1/2 代码均已落地（2026-08-30 代码核实）**
> 范围：AxAgent 能力发现 × 认知编排 × 执行器的一致化能力模型
> 与既有文档关系：
> - `docs/capability-discovery-orchestration-gap-plan.md` — 能力发现与认知编排差距分析与实施复盘（10 类对象、反馈闭环、暴露模式，**已完成**）
> - `docs/COGNITIVE_ROUTER_DESIGN.md` — 三层认知路由设计（L1 域 → L2 簇 → L3 RAR/图谱）
> - 本文档定义**能力本体的统一建模规范**：所有能力对象（Tool/Skill/Workflow/Toolchain/Template/Policy/知识片段/Alias/能力组）共享的元模型、层级结构与关系模型，并给出与 AxAgent 现有护照体系的映射与增量实施顺序。

---

## 一、建模目标

让系统（LLM、编排器、人类开发者）用**一致的方式**理解、发现、加载和执行任何能力，无论其内部复杂度如何。具体收益：

| 收益 | 说明 |
|---|---|
| 发现统一 | 发现引擎不区分对象类型，同一套检索逻辑处理所有能力 |
| 编排统一 | 编排引擎递归展开任何可执行能力：Workflow 含 Skill，Skill 含 Tool |
| 加载统一 | 加载过程一致：元数据 → 完整定义 → 实例化 → 执行 |
| 扩展统一 | 新增能力类型只需继承基类，不影响现有发现/编排逻辑 |
| 人机协同 | 人类开发者用同一模型理解能力结构，降低沟通成本 |

## 二、五条建模原则

1. **统一抽象层（Unified Abstraction Layer）**：所有能力继承基础能力模型（Base Capability Model），拥有相同核心字段与生命周期，使发现引擎、编排引擎、执行器用同一套逻辑处理不同粒度的对象。
2. **递归组合（Recursive Composition）**：Skill 可含 Tool，Workflow 可含 Skill，Workflow 可含 Workflow；支持嵌套组合，且组合关系自身是可发现的元信息。
3. **面向语义而非实现（Semantic-First）**：模型描述能力的"做什么"与"如何交互"，而非"代码怎么写"；LLM 可理解，异语言组件可互操作。
4. **版本化与可演化（Versioned & Evolvable）**：能力可迭代，建模支持版本管理，旧版本在执行期间保持稳定。
5. **可发现性优先（Discoverability-First）**：字段设计首要目标为提升检索与匹配准确率，其次才是执行效率。

---

## 三、统一能力模型层级结构（目标模型）

### 第一层：核心元模型（Core Meta-Model，所有能力共有）

```yaml
BaseCapability:
  id: string                     # 全局唯一标识，如 "skill.weekly_report.v3"
  name: string                   # 人类可读名称，如 "生成周报"
  description: string            # 自然语言描述，面向 LLM 和人类
  version: string                # 语义化版本，如 "1.2.3"
  kind: CapabilityKind           # 枚举：TOOL, SKILL, WORKFLOW, TEMPLATE, TOOLCHAIN, PLUGIN, POLICY, KNOWLEDGE_SNIPPET, ALIAS, CAPABILITY_GROUP
  owner: string                  # 所属团队或个人
  created_at: timestamp
  updated_at: timestamp
  status: CapabilityStatus       # ACTIVE, DEPRECATED, EXPERIMENTAL, DISABLED
  labels: list[string]           # 分类与检索标签，如 ["security", "reporting"]
  tags: map[string]string        # 键值对元数据，如 {"risk_level": "high", "cost_tier": "premium"}
```

这一层是所有能力管理、检索、权限控制的共同基础。

### 第二层：可执行能力模型（Executable Capability Model）

被调用执行的核心对象：Tool、Skill、Workflow、Toolchain。

```yaml
ExecutableCapability extends BaseCapability:
  # 输入输出契约
  input_schema: JSONSchema        # 严格定义输入参数的 JSON Schema
  output_schema: JSONSchema       # 严格定义输出结构的 JSON Schema
  # 执行元信息
  execution:
    mode: ExecutionMode           # SYNC / ASYNC / STREAMING
    timeout: duration             # 最大执行时间，如 "30s"
    retry_policy: RetryPolicy     # 重试策略
    estimated_duration: duration  # 预估执行耗时，用于编排决策
    estimated_cost: Cost          # 预估执行成本（API 费用等）
  # 前提条件
  preconditions:
    - condition: string           # 自然语言描述的条件
      evaluator: string           # 可编程评估表达式或函数引用
  # 权限与安全
  security:
    required_permissions: list[string]
    allowed_contexts: list[string] # 如 ["internal_network", "prod_env"]
  # 依赖关系（用于关联发现）
  dependencies:
    upstream: list[string]        # 通常前置的能力 ID 列表
    downstream: list[string]      # 通常后置的能力 ID 列表
    alternatives: list[string]    # 等价替代能力 ID 列表
    conflicts_with: list[string]  # 互斥能力 ID 列表
```

**各类型特有字段：**

```yaml
Tool extends ExecutableCapability:
  implementation:
    type: ImplementationType      # REST_API, GRPC, LOCAL_FUNCTION, SHELL_SCRIPT, etc.
    endpoint: string              # 对于 API：URL
    method: string                # GET/POST 等
    auth: AuthSpec                # 鉴权方式描述
    request_template: string      # 请求体模板
    response_parser: string       # 响应解析规则

Skill extends ExecutableCapability:
  steps:
    - step_id: string
      capability_id: string       # 引用另一个能力（Tool/Skill/Workflow）
      params: map[string, any]    # 步骤级参数映射
      condition: string           # 可选：执行条件
      on_error: ErrorHandling     # 错误处理策略
  business_rules:
    - name: string
      description: string
      rule_expression: string     # 可执行的规则表达式
  referenced_knowledge: list[string]  # 知识片段 ID 列表

Workflow extends ExecutableCapability:
  dag:
    nodes:
      - node_id: string
        capability_id: string
        params: map[string, any]
        depends_on: list[string]  # 依赖的前置节点 ID 列表
        on_success: string        # 成功后的跳转
        on_failure: string        # 失败后的跳转
    edges:
      - from: string
        to: string
        condition: string         # 条件边
  global_error_handling:
    retry_on_failure: boolean
    fallback_workflow: string     # 失败后回退的工作流 ID
    rollback_steps: list[string]  # 回滚时执行的步骤 ID 列表
  events:
    - event_type: string
      handler: string             # 触发处理的能力 ID
  timeout: duration               # 整个工作流的最大执行时间

Toolchain extends ExecutableCapability:
  tools: list[string]             # 按顺序排列的工具 ID 列表
  context_sharing: ContextStrategy # 工具间上下文传递策略，如 PIPE（管道）
```

### 第三层：非执行能力模型（Non-Executable Capability Model）

不直接执行，但在发现、过滤、编排中起关键作用。

```yaml
Template extends BaseCapability:
  kind: "TEMPLATE"
  template_body: string           # 模板内容，含占位符，如 "扫描 {{target_ip}} 的 {{port_range}}"
  placeholders:
    - name: string
      type: string
      description: string
      default: any
      validation: string          # 正则或自定义验证规则
  instantiates_to: CapabilityKind # 实例化后生成的目标能力类型（SKILL 或 WORKFLOW）
  example_instance: string        # 能力 ID 或内联定义

Policy extends BaseCapability:
  kind: "POLICY"
  policy_type: PolicyType         # FILTER（过滤）, PRIORITY（优先级加权）, SECURITY（安全）
  evaluation_point: EvaluationPoint # PRE_DISCOVERY, POST_DISCOVERY, PRE_EXECUTION
  rules:
    - name: string
      condition: string           # 如 "context.environment == 'internal'"
      action: PolicyAction        # 如 "filter_out: capabilities with label 'cloud_only'"
  priority: int                   # 多策略冲突时按优先级排序

KnowledgeSnippet extends BaseCapability:
  kind: "KNOWLEDGE_SNIPPET"
  content_type: string            # TEXT, JSON, TABLE, MARKDOWN, etc.
  content: string                 # 实际内容
  linked_to: list[string]         # 被哪些能力引用
  valid_until: timestamp          # 可选，过期时间
  source: string                  # 来源引用

Alias extends BaseCapability:
  kind: "ALIAS"
  target_id: string               # 别名指向的真实能力 ID
  alias_type: AliasType           # SYNONYM（同义词）, SHORTHAND（缩写）, LEGACY（历史名）

CapabilityGroup extends BaseCapability:
  kind: "CAPABILITY_GROUP"
  members: list[string]           # 组内包含的能力 ID 列表
  group_strategy: GroupStrategy   # DOMAIN（按领域）, LEVEL（按层级）, CUSTOM
  recursive: boolean              # 是否可递归包含子组
```

### 第四层：组合与关联模型（Composition & Relationship Model）

```yaml
CapabilityRelationship:
  id: string                      # 关系 ID
  source_id: string               # 源能力 ID
  target_id: string               # 目标能力 ID
  relationship_type: RelationshipType:
    - DEPENDS_ON                  # 依赖关系
    - USES                        # 使用关系（能力 A 执行中使用了能力 B）
    - ALTERNATIVE_TO              # 替代关系
    - CONFLICTS_WITH              # 冲突关系
    - PARENT_OF                   # 组合中的父子关系
    - PRECEDES                    # 顺序前置
    - FOLLOWS                     # 顺序后置
    - REQUIRES_KNOWLEDGE          # 需要某知识片段
  weight: float                   # 关系权重，用于检索排序
  context: string                 # 关系描述上下文
  metadata: map[string, string]   # 扩展元信息
```

### 执行时实例化

```yaml
ExecutionInstance:
  capability_id: string           # 能力 ID
  instance_id: string             # 本次执行的唯一实例 ID
  params: map[string, any]        # 填充后的具体参数
  resolved_dependencies: list[ExecutionInstance]  # 递归解析后的依赖实例
  state: ExecutionState           # PENDING, RUNNING, COMPLETED, FAILED
  result: any                     # 执行结果
  logs: list[LogEntry]            # 执行日志
```

---

## 四、AxAgent 现状映射（2026-08-30 已代码核实）

### 4.1 已覆盖（直接复用，禁止重复定义）

| 目标模型 | AxAgent 现有落点 |
|---|---|
| `BaseCapability`（id/name/description/kind/labels） | `CapabilityPassport` trait + `CapabilityPassportDto`（`crates/harness/src/capability.rs`），id 格式 `{kind}:{id}` 一致 |
| 递归组合（Skill 含 Tool / Workflow 含 Skill） | `CapabilityKind` 7 类 + workflow DAG 节点引用 capability_id |
| 语义描述 + 负面场景 | `description` + `negative_scenarios`（检索时语义相似剔除） |
| 五步发现链路 | `CognitiveRouter`（1880 行）：L1 域路由 → L2 簇路由 → L3a RAR 向量 → L3b 图谱路由 → 熔断 → `RoutingDecisionV2` |
| 排序加权（α 语义 / β 成功率 / γ 耗时 / δ 成本） | `DiscoveryWeights` + `CapabilityLevel::derive`（L1-L5，多维加权派生） |
| 分层检索降级 | `CapabilityLayer`（App=Workflow / Task=Skill+Template+Toolchain / Atomic=Tool+Agent+KB），高层 top1 ≥0.45 即命中不降级 |
| 输入契约 + 校验 | `input_schema` + `ToolValidator`（required/type/enum/min/max 校验） |
| 前置条件 | 护照 `preconditions` + `FilterContext.satisfied_conditions/conditions_checked` + 过滤维度十 |
| 权限与安全 | `SecurityLevel`（Public/Internal/Sensitive/Restricted）+ `CallerPermissions` + `Visibility`（Public/SystemOnly/PrivilegedOnly/Hidden）——比目标模型更细 |
| Workflow `dag` / 全局错误处理 | `workflow_types.rs`：`NodeKind`/`RetryConfig`/`CompensationConfig`/`CompensationStrategy`（SkipWithWarning/Rollback/Escalate） |
| `Toolchain`（固定顺序组合） | `CapabilityKind::Toolchain` + `steps: Vec<String>` + `cognitive_execute_toolchain` 确定性执行器（步骤间透传输出，失败短路） |
| `Template` 占位符 | `CapabilityKind::Template` + `PlaceholderDef`（name/type/description）+ `entity_extractor` 正则实体提取（ip/date_range/number/email/url/id） |
| `Policy`（过滤器） | `capability_policies` 表（v128）+ 8 维闸门 + 可注册策略规则，filter 集成 |
| `KnowledgeSnippet` | 护照 `attached_snippets: Vec<KnowledgeSnippet{key, content}>`（能力与信息分离载体） |
| `Alias` | 护照 `aliases: Vec<String>`，retriever 命中别名直接进候选 + 语义加权 |
| `CapabilityGroup` | L2 簇（`capability_clusters.rs`）静态簇路由 |
| `execution.estimated_cost` / `estimated_duration` | 护照 `estimated_cost_usd` / `avg_duration_seconds` + `SessionBudget` 会话预算过滤 |
| 能力等级（成熟度） | `CapabilityLevel`（L1-L5）+ `CapabilityStats`（total_calls/success_rate/熔断状态） |
| 暴露模式 | `CapabilityExposure`（Auto/OnDemand/Managed）+ `CapabilityToolRef`（tool_ref 反查 UnifiedToolRegistry 注入 chat_tools） |
| 运行时注册表 | `CapabilityRegistry` + `ServiceDefinition`（三件套：ServiceDefinition/Provider/Consumer，可逆注册 `EffectHandle`） |

### 4.2 剩余缺口（本文档实施对象）

| 级别 | 缺口 | 目标模型对应 | 现状说明 |
|---|---|---|---|
| **P0** | 版本化字段（version/owner/created_at/updated_at） | `BaseCapability.version/owner/created_at/updated_at` | ✅ 已完成 2026-08-30（代码核实）：`CapabilityPassportDto` 已含 7 字段（version/owner/created_at/updated_at/output_schema/execution_mode/timeout_ms，均为 `Option` 或带 default 向后兼容）+ trait 默认方法 `version()/owner()/output_schema()/execution_mode()/timeout_ms()` + `to_passport_dto` 回填 + 前端 `capability.ts` 同步 |
| **P0** | 输出契约 `output_schema` | `ExecutableCapability.output_schema` | ✅ 已完成 2026-08-30（代码核实）：`output_schema: Option<serde_json::Value>` 字段就位，trait 默认方法 + 前端同步；编排器静态校验"下游入参 = 上游输出"仍待接线（Phase 0 验收仅要求字段存在） |
| **P0** | 执行模式 `execution_mode`（SYNC/ASYNC/STREAMING）+ 工具级 timeout | `ExecutableCapability.execution.mode/timeout` | ✅ 已完成 2026-08-30（代码核实）：`execution_mode: ExecutionMode`（Sync/Async/Streaming）+ `timeout_ms: Option<u64>` 字段就位，护照级声明使认知编排不再仅靠 kind 猜执行路径；`clamp_mode_for_kind` 实际消费护照 `execution_mode` 分发仍待后续接线 |
| **P1** | Tool 实现契约（type/endpoint/method/auth/request_template/response_parser） | `Tool.implementation` | ✅ 已完成 2026-08-30：`ImplementationType` + `ToolImplementation` + 护照 `implementation` |
| **P1** | Skill 步骤结构化（step 级 params/condition/on_error） | `Skill.steps` | ✅ 已完成 2026-08-30：`SkillStep` + 护照 `skill_steps`（`steps` 保留给 Toolchain） |
| **P2** | 独立关系模型 `CapabilityRelationship`（type/weight/context/metadata） | 第四层 | ✅ 已完成 2026-08-30：v129 表（复合主键）+ entity + repo 物化 + 检索 2 跳 BFS 权重衰减 |
| **P2** | Template 字段层（template_body/instantiates_to/example_instance） | `Template` | ✅ 已完成 2026-08-30：字段就绪；实例化执行器挂起（待真实场景定产物格式） |
| **P2** | kind 枚举对齐（PLUGIN/POLICY/ALIAS/CAPABILITY_GROUP 是否进枚举） | `BaseCapability.kind` | ❌ 已评估不实施：功能已由 aliases 字段/policies 表/L2 簇/source=Plugin 实现，加枚举值违反「禁止重复定义」 |

---

## 五、工程落地要点（与 AxAgent 现实适配）

| 目标模型的存储设计 | AxAgent 适配决策 | 理由 |
|---|---|---|
| 文档型数据库（MongoDB）存能力定义 | **不引入**。能力护照 = 内存索引 + 向量库 metadata（RAR）+ SQLite 落 DB（`capability_stats` v127 / `capability_policies` v128） | 桌面单机，无分布式需求；引入 Mongo 违反"一切皆插件"轻量原则 |
| 图数据库（Neo4j）存关系 | **不引入**。`CapabilityRelationship` 落地为 SQLite 表 + 内存邻接，一跳扩展用 DTO 内联，多跳遍历用内存 BFS | 规模（数百能力）下图库无收益 |
| 向量数据库（Pinecone）存 Embedding | 复用现有 RAR 向量检索（kb 知识图谱 / 本地 llama.cpp bge-m3） | 已有基础设施 |
| 缓存层（Redis） | **不引入**。护照在内存（RwLock HashMap）+ 启动快照 | 无跨进程共享需求 |

执行器不直接操作原始模型：`ToolContext`（工作目录/会话/权限/中止信号/回滚栈）+ `ToolResult`（content/truncated/is_error/metadata/duration_ms/progress）即 `ExecutionInstance` 的 AxAgent 形态。

---

## 六、分期实施计划

### Phase 0 — 护照版本化 + 执行契约（P0，已完成 2026-08-30 代码核实）

| 步骤 | 改动 | 文件 |
|---|---|---|
| 0.1 | `ExecutionMode` 枚举（Sync/Async/Streaming，serde snake_case） | harness/src/capability.rs |
| 0.2 | `CapabilityPassportDto` 增：`version`/`owner`/`created_at`/`updated_at`/`output_schema`/`execution_mode`/`timeout_ms`（serde default 向后兼容） | harness/src/capability.rs |
| 0.3 | `CapabilityPassport` trait 增默认方法：`version()`/`owner()`/`output_schema()`/`execution_mode()`/`timeout_ms()` | harness/src/capability.rs |
| 0.4 | `to_passport_dto` 回填新字段 | harness/src/capability.rs |
| 0.5 | 前端 `src/types/capability.ts` 同步 `CapabilityPassportDto` + `ExecutionMode` | src/types/capability.ts |
| 0.6 | 验证：`cargo check`（零警告）+ `tsc --noEmit` + `dprint fmt` | — |

验收：护照 DTO 含 7 个新字段；旧序列化数据反序列化不破坏（default 兜底）；前端类型与后端 DTO camelCase 对齐。

### Phase 1 — Tool 实现契约 + Skill 步骤结构化（P1）

- `ToolImplementation` 结构（type/endpoint/method/auth/request_template/response_parser），护照 `implementation: Option<ToolImplementation>`
- `SkillStep` 结构（step_id/capability_id/params/condition/on_error），`steps` 从 `Vec<String>` 平滑迁移（旧字符串兼容）
- 前端类型同步；`cognitive_execute_toolchain` 消费结构化步骤

### Phase 1 — Tool 实现契约 + Skill 步骤结构化（P1，已完成 2026-08-30）

| 步骤 | 改动 | 文件 |
|---|---|---|
| 1.1 | `ImplementationType` 枚举（local_function/rest_api/grpc/shell_script/mcp/tauri_command）+ `ToolImplementation` 结构（impl_type/endpoint/method/auth/request_template/response_parser） | harness/src/capability.rs |
| 1.2 | `SkillStep` 结构（step_id/capability_id/params/condition/on_error）；DTO 新增 `skill_steps: Vec<SkillStep>`（Skill 专用）——`steps: Vec<String>` 保留给 Toolchain（`cognitive_execute_toolchain` 消费，零破坏） | harness/src/capability.rs |
| 1.3 | trait 默认方法 `implementation()` / `skill_steps()`；DTO `implementation`/`skill_steps` 字段 + Default + to_passport_dto 回填 | harness/src/capability.rs |
| 1.4 | 前端 `capability.ts`：ImplementationType/ToolImplementation/SkillStep 类型 + DTO 字段（camelCase）；`browserMock.ts` 补字段 | src/types/capability.ts、src/lib/browserMock.ts |
| 1.5 | `state.rs` 6 处全字段字面量补字段（幂等脚本清理） | src/init/state.rs |
| 1.6 | 验证：`cargo check -p axagent` + `tsc --noEmit` + `dprint fmt` + `cargo fmt` | — |

踩坑：Python 脚本判断 Rust 字面量结束行用 `endswith("}")`，但 vec 元素以 `},` 结尾（以逗号收尾），恒不匹配 → 边界失效导致重复插入；修复为「strip 后以 `},`/`};`/`});` 结尾且缩进 **严格小于** 锚点缩进」，并对 agent 字面量（sub_category if-else 内部 `},` 与锚点同缩进）幂等清理至 6/6/6。

### Phase 2 — 关系模型 + Template 字段 + kind 对齐（P2，已完成 2026-08-30）

| 步骤 | 改动 | 文件 |
|---|---|---|
| 2.1 | `RelationshipType` 枚举（8 种）+ `CapabilityRelationship` 序列化 DTO | harness/src/capability.rs |
| 2.2 | v129 migration `capability_relationships` 表（复合主键 source_id+target_id+relationship_type，规避 SQLite BIGINT PRIMARY KEY 不自增）+ entity + dao repo（sync_from_passports 清空重建物化 / list_relations / relations_for / dependencies_of，3 单测） | dao/src/migrations/v129_capability_relationships.rs、entities/src/capability_relationships.rs、dao/src/repo/capability_relationship.rs |
| 2.3 | 启动物化：`register_capability_passports` 末尾合并业务+系统护照 → `sync_from_passports`（upstream→DEPENDS_ON、downstream→FOLLOWS）；`register_system_capabilities` 改为返回 passports | src/init/state.rs |
| 2.4 | 检索多跳：`expand_dependencies` 从一跳升级为 **2 跳 BFS + 每跳 ×0.6 衰减**（第 1 跳保底 0.4 / 第 2 跳 0.24），仍以内存护照图为源（零 DB 依赖，关系表不参与检索热路径） | crates/tools/src/capability_retriever_impl.rs |
| 2.5 | Template 字段层：DTO 加 `template_body`/`instantiates_to`/`example_instance`（serde default 兼容）+ Default + to_passport_dto 回填 + state.rs 6 处字面量（幂等脚本 6/6/6） | harness/src/capability.rs、src/init/state.rs |
| 2.6 | 前端：capability.ts 加 CapabilityRelationship/RelationshipType + DTO 3 字段；browserMock.ts 同步 | src/types/capability.ts、src/lib/browserMock.ts |
| 2.7 | **kind 枚举对齐（P2-③）：已评估，不实施**——ALIAS/POLICY/能力组/插件功能已分别由 aliases 字段、capability_policies 表（v128）、L2 簇、source=Plugin 实现，加枚举值会造成"同一功能两种表达"，违反项目「禁止重复定义」铁律 | — |

**P2-② 遗留（挂起）**：Template 实例化执行器（填充占位符 → 生成 Skill/Workflow 定义）。字段层已就绪；执行器待真实"模板→Skill"场景驱动，产物格式（Skill YAML / Workflow JSON）届时再定，避免凭空设计。

---

### 审计结论（2026-08-30 代码核实 + 编译/单测/clippy/tsc 全绿）

- Phase 0/1/2 **代码均已落地**，全部验证通过：`cargo test -p axagent-dao --lib` 95 passed；`cargo clippy`（harness/tools/dao，`-D warnings`）零警告；`cargo check -p axagent` 干净；`tsc --noEmit` exit 0；`cargo fmt` 0 diff。
- 审计中修复 5 处缺陷：`capability_relationship.rs` 测试逻辑错误、2 处 `field_reassign_with_default`、`collapsible_if`、`rar_router.rs:1295` E0063（缺 12 个新增必填 DTO 字段）。
- 文档 §4.2 三行 P0 缺口已就地校正为「✅ 已完成」。

## 七、待办清单（下一期，触发条件驱动）

> 下列 4 项均**不在** Phase 0/1/2 交付/验收范围内；但已按"触发条件驱动、不破坏既有行为"原则落地**可调用机制 + 单测**（2026-08-30）。
> 机制在触发条件未命中时为 no-op（当前所有护照 `execution_mode` 默认 `Sync`、`output_schema` 未填充、`sync_from_passports` 不写运行时关系），故不改变既有功能与构建。
> 仅当真实触发条件出现时机制才生效；深度治理（产物格式、下游引用修复等）仍留给触发场景。任务系统 4 条（#1/#2/#4/#3）已置为进行中并与本文档对齐。

| # | 待办项 | 已落地机制（2026-08-30） | 触发条件（命中才生效） | 任务 ID |
|---|---|---|---|---|
| ① | 护照 `execution_mode` 接入 dispatcher 实际分派 | `resolve_execution_mode(passport_mode, kind)`（`capability.rs`）按 kind 安全降级；在 `expand_dependencies` 发现边界对每候选解析并 debug 记录 | 认知编排需按护照声明（而非 kind）决定 Sync/Async/Streaming 分派 | #1 |
| ② | `output_schema` 上下游契约静态校验 | `validate_dependency_chain_compatibility` + `check_edge_compatibility`（`capability_retriever_impl.rs`），并在 `expand_dependencies` 扩展后运行时告警；schema 未就绪时为 no-op | 编排器需在执行前静态校验「下游入参 = 上游输出」以避免运行时类型错配 | #2 |
| ③ | Template 实例化执行器（`template_body` → Skill/Workflow） | `instantiate_template(passport)` 纯函数（`capability.rs`）：填充 `{{占位符}}`、复用现有 `SkillStep`、输出 `InstantiatedTemplate`；不发明产物格式 | 出现真实 Template→Skill/Workflow 实例化需求（禁止凭空设计产物格式） | #4 |
| ④ | `superseded` 版本淘汰/替换治理链路 | `RelationshipType::SupersededBy` 变体 + `mark_superseded`/`superseded_by`（`capability_relationship.rs`，TEXT 列无需 migration） | 出现能力版本淘汰/替换（旧版 superseded）的真实治理需求 | #3 |

## 八、相关文件

- `crates/harness/src/capability.rs` — 护照契约与 DTO（本文档 Phase 0 主战场）
- `crates/harness/src/cognitive_router.rs` — 三层认知路由
- `crates/harness/src/capability_retriever.rs` / `capability_ranker.rs` / `capability_filter.rs` — 发现链路
- `crates/harness/src/workflow_types.rs` — Workflow 护照实现与 DAG 类型
- `src/types/capability.ts` — 前端权威类型（手工对齐，非 ts-rs 生成）
- `docs/capability-discovery-orchestration-gap-plan.md` — 前一阶段差距分析与实施复盘
