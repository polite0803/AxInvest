# PROPOSAL：WorkflowNode 开放化 —— 从封闭 enum 到动态注册模型（P6.0-①）

> 状态：in-progress（阶段0/1/2 完成，阶段3 待启动） ｜ 日期：2026-11-17 ｜ 对应版本：v3.0.0+（未定版，待确认）
>
> 定位：P6.0 六项前置中的第 ① 项，是 P6.3（工作流编辑器迁出）的前置；对 P6.1 股票线改动面为 0
> 估工：12–18 人日（沿用 PLAN §6.1 口径，本方案不改估工数，只把它拆成可执行阶段）
> 关联：`PLAN-shell-plus-plugins.md` §0.8.3-1 / §0.8.4 / §6.1 / 注15 / §207.2
> 盘上实读位点依据：2026-11-17 专项探索报告（本方案 §2 逐条列出，口径见 §2.0）

---

## 0. 一句话目标与验收判据

**目标**：把 `WorkflowNode` 从「32 变体的编译期封闭 enum」改造成「类型字符串 + 契约声明 + 运行期注册表」模型，使**新增一个工作流节点类型时，主壳代码零修改**——节点契约由实现方（功能包 / crate）自行注册。

**验收判据**（沿用 PLAN §6.1 与 §964 原文）：
1. 新增一个节点类型（含其 config、执行器、前端面板、i18n）**不改主壳任何既有文件**，只在功能包侧新增文件 + 调用注册 API；
2. 引擎生产区 `grep -i stock` = 0（本项已由 P6.0-⑤ 完成，本方案不得回退）；
3. 现有 32 种节点的行为、序列化格式（JSON `{"type": "...", ...}`）、DAG 语义**逐字不变**——这是一次内部结构重构，不是行为变更。

**非目标**（明确不做，防止越界）：
- 不迁任何宿主命令、不改股票 / OPC 业务节点（属 P6.1 / P6.2）；
- 不改 `WorkEngine` 角色定性（P6.0-②，另案）；
- 不改前端 tab/route 数据驱动（P6.0-④，另案）；
- 不追求「节点执行器走能力注册表」（P6.0-⑥ 单轨化，另案）——本方案只解决**节点类型开放**这一件事。

---

## 1. 为什么必须改：现状的结构性缺陷

### 1.1 封闭 enum 穿透全域

`WorkflowNode` 定义于 `axagent-core/crates/harness/src/workflow_types.rs`（enum 本体约 `:1366`，32 变体）。它是 `#[serde(tag = "type", rename_all = "camelCase")]` 的 tagged enum，每个变体携带**字段各异的 config 结构体**（`TriggerNode` / `AgentNode` / `StorageNode` …）。

种子数据、OPC、认知路由、进化器全域都按这个 enum 构造 DAG（如 `seed_stock_analysis.rs` 单文件 118 处 `WorkflowNode::` 引用）。任何新节点类型都必须回到 harness 改 enum、重新编译整条依赖链——功能包无法自带节点类型。这正是 L2 能力插件化里 `workflow.node` 被判 K3 失败（PLAN §286 / §337 / §1197）的根因：键是 `&'static str` 穷举，不是运行期可贡献的接缝。

### 1.2 8 类位点 / 逐条枚举（口径说明）

PLAN 记录过三种数字，容易打架，先统一口径：

| 口径 | 数字 | 出处 |
|---|---|---|
| 「新增节点要改 8 类位点」 | 8 类（每类内部可能有多份表） | PLAN §0.8.3-1 / §6.1 |
| 「编译强制穷举位点」 | 旧记 13 处 / 9 文件；2026-11-17 实读修正为 **9 处严格 E0004 / 4 文件**（若把 base_id/base/base_mut/base_output_var 四个 accessor 全算，则更多） | PLAN 注15（:1168） vs 本方案 §2 实读 |
| 「机械位点」 | 至少 16 个 | PLAN §207.2 |

⇒ **数字差异源于「穷举」的定义，不是事实冲突**。本方案不纠结单一数字，改为**把每一处位点逐条列出**（§2），读者可自行按任一口径计数。验收时以「§2 清单全部收口、新增节点不改主壳」为准。

---

## 2. 位点全清单（2026-11-17 盘上实读，行号仅作定位锚，动手前必须复核）

> ⚠ 行号会随本批改动漂移；以「符号名 + 文件」为准，行号仅供本次定位。

### 2.1 Rust harness 侧（数据模型与通用逻辑）

| # | 位点 | 文件 | 性质 | 改造动作 |
|---|---|---|---|---|
| 1 | `enum WorkflowNode` 本体（32 variant + 各自 config） | `axagent-core/crates/harness/src/workflow_types.rs`（~:1366） | 封闭源 | 改为动态结构（见 §3） |
| 2 | `base_id()` match self | 同上（~:1421） | 严格穷举 | 收口到统一 base 字段访问 |
| 3 | `base()` match self | 同上（~:1459） | 严格穷举 | 同上 |
| 4 | `base_mut()` match self | 同上（~:1497） | 严格穷举 | 同上 |
| 5 | `base_output_var()` match self | 同上（~:1558） | 严格穷举 | 同上 |
| 6 | `adjust_position()` `match &mut node` | `axagent-core/crates/harness/src/assembly_builder.rs`（~:385） | 严格穷举 | 收口到统一 base 字段 |
| 7 | `set_base_id()` `match self` | 同上（~:426） | 严格穷举 | 同上 |
| 8 | `deserialize()` 32 臂 match | `axagent-core/crates/harness/src/workflow_node_deserializer.rs`（~:13，match 在 :31） | 功能必改（有兜底） | 改为注册表查表 |
| 9 | `unknown_variant` 32 名白名单 | 同上（~:64） | 功能必改（双表） | 与注册表合并，消除双表 |

### 2.2 Rust rt-workflow 侧（执行与校验）

| # | 位点 | 文件 | 性质 | 改造动作 |
|---|---|---|---|---|
| 10 | `node_type_name()` match node（32 臂） | `axagent-runtime/crates/rt-workflow/src/work_engine/node_executor_trait.rs`（~:233） | 严格穷举 | 节点契约提供 type_name |
| 11 | `node_kind()` match node（32 臂分组） | 同上（~:271） | 严格穷举 | 节点契约提供 kind |
| 12 | `get_node_type()` match node | `axagent-runtime/crates/rt-workflow/src/work_engine/engine/schema_validator.rs`（~:104，另 :43-64 的 9 契约 + 兜底） | 严格穷举 | 节点契约提供 config schema |
| 13 | `dispatcher::init_builtin()` 27 个 `register(...)` | `axagent-runtime/crates/rt-workflow/src/work_engine/dispatcher.rs`（~:80-110） | 编译期写死 | 改为遍历注册表自动注册 |
| 14 | 带依赖 executor 的构造与 `register_arc`（Agent/Llm/Condition/LlmClassifier/Switch 等） | `axagent-runtime/crates/rt-workflow/src/work_engine/engine/mod.rs`（~:1114-1124、~:1340-1351） | 编译期写死 | 改为工厂注册（provider 由引擎注入工厂） |

> 非穷举但有局部匹配的位点（`dispatcher.rs:465 build_node_input_snapshot`、`workflow_port_axioms.rs:309`、`plan_compiler.rs:297`、`dag_store.rs`、`output_builder.rs`、`workflow_fanout_audit.rs:78-131`、`workflow_ai_diagnose.rs:117` 等）**带 `_` 兜底或单变体匹配**，新增类型时不会编译报错，但会静默降级（走兜底分支）。⇒ 它们是 §2.1 改造后的**回归重点**：动态模型下这些分支必须仍能对未知/新增类型给出正确默认行为，不能因拿到 `Value` 就崩溃。

### 2.3 前端

| # | 位点 | 文件 | 性质 | 改造动作 |
|---|---|---|---|---|
| 15 | `NODE_TYPE_MAP`（后端 32 类型 + 2 装饰类型） | `src/components/workflow/types/workflow.types.ts`（~:1005） | `Record<string,…>` 不强制穷举，静默降级 | 改为运行期注册（后端契约下发或前端注册表） |
| 16 | `NodePropertyPanel` 的 32 case + default | `src/components/workflow/Panels/RightPanel.tsx`（~:52，default ~:320） | 有 default，静默降级 | 面板按 type 查注册表 |
| 17 | 32 个 PropertyPanel re-export | `src/components/workflow/Panels/PropertyPanels/index.ts`（~:3-35） | 注册表 | 插件面板自带导出 |
| 18 | `nodeTypes` 段 11 语言 key | `src/i18n/locales/*.json`（各 ~:9547/9556） | 数据 | 标签由节点契约随类型下发 |

> 前端是**最危险的那一半**（PLAN 注15）：4 张表全带 default 兜底 ⇒ 新增类型不报错、只是静默缺面板/缺文案。改造必须让「未注册类型」在前端**显式可见**（兜底渲染成「未知节点类型 + type 字符串」，而非空白）。

---

## 3. 目标模型设计

### 3.1 核心数据结构（harness）

把 tagged enum 替换为**统一外壳 + 类型字符串 + 原始 config**：

```rust
/// 工作流节点的运行期统一表示（替代封闭 enum）。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowNode {
    /// 节点类型字符串（原 enum tag），如 "agent" / "storage" / "thirdPartyXxx"
    pub node_type: String,
    /// 节点通用字段（id / title / position / retry / enabled / hooks …）
    #[serde(flatten)]
    pub base: WorkflowNodeBase,
    /// 类型专属配置；布局/持久化时保留原始 JSON，执行时由注册表解析成强类型 config
    pub config: serde_json::Value,
}
```

关键点：`config` 用 `serde_json::Value` **无损保留**现有 32 种 config 的 JSON 形状 ⇒ 现存工作流定义、DB 中的 DAG、种子模板的序列化字节**完全不变**（验收判据 3）。

### 3.2 节点契约 trait（注册表条目）

```rust
/// 一个节点类型的完整契约：类型标识 + 分类 + config 解析 + 面板元数据。
pub trait WorkflowNodeDescriptor: Send + Sync {
    fn type_name(&self) -> &'static str;
    fn kind(&self) -> NodeKind;
    /// 把原始 config 解析成该类型期望的形状；校验失败返回可读错误。
    fn parse_config(&self, raw: &serde_json::Value) -> Result<(), String>;
    /// config 的 JSON Schema（供 schema_validator 与前端表单生成）
    fn config_schema(&self) -> serde_json::Value;
}
```

### 3.3 注册表（两处，各司其职）

| 注册表 | 位置 | 作用 | 谁往里注册 |
|---|---|---|---|
| `NodeDescriptorRegistry` | harness（foundation，零 axagent 依赖） | type_name → descriptor（kind / parse / schema / i18n key） | 各节点类型定义方（含功能包） |
| Executor 注册表 | 既有 `rt-workflow/dispatcher.rs` 的 `HashMap<&'static str, Arc<dyn NodeExecutorTrait>>` | type_name → 执行器 | 改为遍历 descriptor + 工厂 |

`WorkEngine::init` 时：遍历 `NodeDescriptorRegistry` ⇒ 对每个 descriptor 装配其 executor（带 provider 的用工厂闭包注入）⇒ 注册进 dispatcher。新增节点类型 = 定义 descriptor + executor + 调用一次注册 API，主壳零改。

### 3.4 强类型 config 的归属（消除"丢失类型安全"的顾虑）

动态外壳不等于全仓放弃类型安全。做法是**在 descriptor 边界做一次解析**：

- 各节点原有的 `XxxNodeConfig` 结构体**保留**，只是从"enum 变体的载荷"变成"节点模块内部的强类型 DTO"；
- 执行器 `execute()` 入口用 `serde_json::from_value::<XxxNodeConfig>(&node.config)` 还原成强类型（失败返回校验错误）——这与今天 deserializer 里 `try_from_value!` 做的事**等价**，只是从全局 match 移到各节点自己手里；
- 结果：执行器内部仍是强类型；只有跨注册表的边界是 `Value`。

### 3.5 前端契约

- 后端提供一个**节点类型清单接口**（或从 descriptor 注册表生成静态清单），下发 `{type, kind, schema, labelKey, panel}`，前端 `NODE_TYPE_MAP` / RightPanel / i18n 改为数据驱动；
- 未注册类型的兜底：渲染「未知节点类型 `xxx`」+ 原始 config 的可编辑 JSON，**不静默空白**。

---

## 4. 分阶段迁移路径（每阶段可独立验收、可回退）

> 纪律：这是**11–18 人日**的重构，必须分阶段；阶段内用定点 `cargo check -p`，**全部阶段完成后**才跑一次全量门禁（AGENTS.md 铁律）。

### 阶段 0 —— 契约与清单冻结（0.5 人日）
- 逐条复核 §2 全部位点的最新行号与符号名；
- 冻结 32 种节点的 JSON 序列化样本（从现有测试/种子里导出），作为**行为不变**的回归基线；
- 产出：更新本方案 §2 行号；落 `docs/audits/` 一份位点快照。

### 阶段 1 —— 引入动态外壳 + descriptor trait（并行期，enum 与外壳共存）（2–3 人日）
- 在 harness 新增 `WorkflowNode` 动态结构与 `WorkflowNodeDescriptor` / 注册表；**暂不替换**现有 enum，先让两者通过 `From` 互转；
- 为 32 种节点各写一个 descriptor（可宏生成样板），注册进 `NodeDescriptorRegistry`；
- 验证：新增单元测试断言「enum → 外壳 → enum」往返序列化字节逐字相等（32 种全覆盖）；定点 `cargo check -p axagent-harness`；
- 回退：删新增文件即可，enum 未动。

### 阶段 2 —— 反序列化注册表化 + 序列化形状锁定（2–3 人日）
> **2026-11-17 实况订正**：原计划本阶段收口 6 处 accessor，但阶段1 明确 enum 与外壳**共存**、
> enum 仍是权威 ⇒ accessor 必须返回 enum 变体内字段的引用（`base_mut()` 尤其如此，
> 无法经 JSON 往返返回 `&mut`）。**accessor 6 处 + adjust_position/set_base_id 的收口
> 后移到阶段6**：删除 enum、动态外壳成为唯一权威时它们自然消失，不需要"假装收口"。
> 本阶段交付可独立验收的两件事：
- `workflow_node_deserializer.rs` 的 `unknown_variant` 32 名手写白名单改为查
  `NodeDescriptorRegistry`（`OnceLock` 懒初始化），消除"match 臂 + 名单"双表；
  合法类型仍走原强类型 wrapper 解析，行为逐字不变；
- **修复阶段1 遗留缺陷**：`WorkflowNodeDyn.node_type` 原受 `#[serde(rename_all="camelCase")]`
  影响输出 `nodeType`（与 enum 的 `"type"` 不一致）——阶段1 的 `TryFrom` 桥接手工拆装 JSON
  绕过了自身 Serialize 故未暴露；本阶段加 `#[serde(rename="type")]` `#[ts(rename="type")]`
  并用 32 标签测试锁住形状；
- 新增 32 标签「外壳无损」测试 + 注册表/match 同集测试（7 个 workflow_node 测试全绿）；
- 验证：`cargo check -p axagent-harness --all-targets` / `cargo clippy -p axagent-harness
  --all-targets -- -D warnings` / `cargo test -p axagent-harness --lib workflow_node` 三绿；
- 阶段0 审计 §4 后移的「32 变体完整合法实例往返比对」进一步后移到阶段6（理由：届时
  `config: Value` 直接是权威，比对才有意义；共存期逐个猜必填字段收益低）；
- 回退：git revert 本阶段提交。

### 阶段 3 —— 收口 rt-workflow 侧（node_type_name / node_kind / schema_validator）（2–3 人日）
- 三处 match 改为从 descriptor 读 type_name / kind / schema；
- 验证：dispatcher 能跑通现有工作流（用现有 e2e / 单测）；定点 `cargo check -p axagent-rt-workflow`；
- 回退：git revert。

### 阶段 4 —— dispatcher 自动注册 + executor 工厂化（3–4 人日）
- `init_builtin` 改为遍历注册表；带 provider 的 executor 改为工厂闭包，provider 由 `WorkEngine::init` 注入；
- 这是**最关键的一步**：完成后「新增节点不改主壳」在 Rust 侧成立；
- 验证：新增一个**测试专用节点类型**（只写 descriptor + executor + 一次注册，不改任何主壳文件），跑通一次含该节点的工作流 ⇒ 这就是验收判据 1 的可执行证明；
- 回退：git revert。

### 阶段 5 —— 前端数据驱动（3–4 人日）
- 节点清单接口 + `NODE_TYPE_MAP` / RightPanel / PropertyPanels / i18n 改为按清单渲染；
- 未注册类型显式兜底；
- 验证：新增测试节点类型的前端面板**无需改主壳**即可显示；`npm run typecheck` + `npm run test:run`；前端 lint（oxlint）；
- 回退：git revert。

### 阶段 6 —— 删除旧 enum + 全量门禁收官（1–2 人日）
- 确认全仓零 `WorkflowNode::` 强类型构造后，删除旧 enum 与共存互转代码；
  §2.1 的 6 处 accessor + `adjust_position` / `set_base_id` 随 enum 删除自然收口
  （阶段2 已说明后移理由）；同时补 32 变体完整合法实例往返比对（阶段0 审计 §4 后移项）；
- 按 AGENTS.md 铁律：三锚同批改（capability-register.json + PLAN §0.7.2 镜像行）、doc-refs 行号订正、single-source 回正；
- 全量门禁（CI 口径，只此一次）：
  ```
  cargo fmt --all --check
  cargo clippy --workspace --all-targets --all-features -- -D warnings
  __TAURI_WORKSPACE__=true cargo test --workspace
  node scripts/check-capability-register.mjs
  node scripts/check-doc-refs.mjs
  node scripts/check-single-source-facts.mjs --ci
  node scripts/check-component-neutrality.mjs
  npm run format:check && npm run typecheck
  ```

---

## 5. 风险与对策

| 风险 | 说明 | 对策 |
|---|---|---|
| **序列化字节漂移** | 最高风险：32 种节点的 JSON shape 必须逐字不变，否则存量工作流 / DB / 种子全崩 | 阶段 0 冻结样本基线；阶段 1/2 每步字节级回归比对；`config: Value` 无损透传 |
| **数百处强类型构造改写** | 全仓 `WorkflowNode::Xxx { config }` 构造/字段访问很多（seed 118 / workflow_types 128 / template 94 / assembly 78 / trait 62 …） | 阶段 1 共存 + `From` 互转，把改写摊到阶段 2–6 逐步做；能保留强类型 config 的地方保留（§3.4） |
| **静默降级（前端）** | 前端兜底会让新类型"不报错但缺面板" | 阶段 5 强制显式兜底（未知类型可见）+ 自动化断言 |
| **clippy 连锁** | 改注释/删 match 可能触发 `doc_lazy_continuation` / `if_same_then_else` / `manual_range_contains` 等（AGENTS.md 实测） | 阶段内定点 clippy `-D warnings`；合并分支前先确认新形态 lint-clean |
| **三锚 / doc-refs 漂移** | 改 harness / engine / init 文件必触发（交接书 §三） | 阶段 6 按交接书清单逐项回正，只改一头会红 |
| **与 P6.0-②/⑥ 交叉** | WorkEngine 角色、执行器单轨化另有专案 | 本方案只碰"节点类型开放"，阶段 4 的工厂化为⑥留接口，不替它实现能力注册表接线 |

---

## 6. 与前后置的关系

- **前置**：P6.0-②（WorkEngine 角色定性）建议先定，因为阶段 4 的工厂注入方式与"引擎是底座还是功能包"相关；若不先定，阶段 4 按"引擎仍是底座"实现，②后续换轨时需适配工厂接口（成本可控）。
- **后置**：P6.3（工作流编辑器迁出）直接依赖本项——本项不完成，编辑器功能包无法自带节点类型。
- **无关**：P6.1 股票线（只用现有 32 变体，改动面 0，PLAN 注15 已证）。

## 7. 决策请求

本方案**仅为立项与设计**，未改任何代码。请评审：
1. §3.1 的「动态外壳 + `config: Value` + 边界解析」模型是否认可（这是丢失全局编译期类型安全 vs 保留执行器内强类型的取舍点）；
2. §4 的 7 阶段拆分与阶段 4 的"测试节点类型"验收法是否认可；
3. 是否先做 P6.0-②（WorkEngine 角色定性）再启动本项，还是按"引擎仍是底座"直接开工。
