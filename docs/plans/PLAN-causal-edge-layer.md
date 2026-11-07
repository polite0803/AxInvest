> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# 因果边层（Causal Edge Layer）实施方案

> 来源：laap-AGI 认知分析的落地结论，见 `docs/laap-agi-移植评估.md`
> 创建日期：2026-09-04
> 目标：在 AxAgent 知识图谱中补上「因果边」语义层——带方向、强度、时间延迟和置信度的关系边
> 代码许可：仅吸收教科书级算法思想（贝叶斯增量更新、Welford 在线方差、BFS 链传播），不复制 LAAP 任何源码

---

## 一、目标与范围

### 1.1 要解决的问题

当前 `knowledge_relations` 只有「相关」语义（`contains`、`follows`），无法表达：

- 「调用 `Read` 之后，平均 1.8 秒会调用 `Edit`」——工具序列的时间因果
- 「用工具 X 处理 Y 类任务，14 次里成功 11 次」——带样本量的强度
- 「为什么推荐这一步」——可解释的因果依据

### 1.2 明确的非目标

| 不做 | 理由 |
|------|------|
| 移植 LAAP 任何 Python 代码 | Rust/Tauri 技术栈；且核心引擎为 BSL 1.1 许可 |
| 量子因果引擎 | 本质是余弦相似度，search crate 已有更成熟的向量检索 |
| 情绪 / 情感 / 激素系统 | 字符串查表，对任务型 Agent 无收益 |
| PC 算法因果发现 | 定向逻辑有 bug；且缺少密集数值观测数据场景 |
| 新建数据表 | `knowledge_relations` 字段已足够 |

### 1.3 技术约束（已勘察确认）

| 约束点 | 现状 | 影响 |
|--------|------|------|
| trajectory 是否依赖 dao | **否**（`dao` 仅在 dev-dependencies） | 必须用 sea-orm + `axagent-entities`，不能调 `dao::repo::knowledge_graph` |
| `axagent-entities` 是否导出 `knowledge_relations` | **是**（`entities/src/lib.rs:47`） | 可直接用实体模型 |
| `knowledge_relations` 唯一索引 | **无**（仅 source/target/source_type 普通索引） | **不能用 `on_conflict` upsert**，需 read-then-write |
| 现有 `upsert_relation()` | `dao/src/repo/knowledge_graph.rs:583` | **有缺陷**：`on_conflict` 键是每次新生成的 `rel_{gen_id()}`，冲突永不触发，实为纯 insert；且硬编码 `knowledge_base_id=""`、`properties=None`。**不可用于本方案，需自建写入逻辑** |
| `properties` 列类型 | SQLite 为 TEXT，PG 为 JSONB | 实体定义为 `Option<Json>`，读写按 `serde_json::Value` 处理 |
| DB 连接范式 | `Arc<DatabaseConnection>`（`storage.rs:32`） | 沿用 |
| Feature flag 机制 | 字符串常量 + `is_enabled("NAME")`（`harness/src/feature_flag_provider.rs`） | 用 `"CAUSAL_EDGES"` |
| 模块注册范式 | `mod xxx;` + `pub use xxx::{...}`（`trajectory/src/lib.rs`） | 沿用 |
| 最新 migration | v133 | 本方案**不新增 migration** |

---

## 二、数据模型

### 2.1 字段复用映射

不新增表、不新增列，全部复用 `knowledge_relations`（`crates/entities/src/knowledge_relations.rs:10-27`）：

| 字段 | 用途 | 取值 |
|------|------|------|
| `relation_type` | 边类型标识 | 固定 `"causes"` |
| `source_entity_id` | 因 | URI 式字符串，见 2.2 |
| `target_entity_id` | 果 | URI 式字符串，见 2.2 |
| `weight: f64` | 因果强度 | `[0.0, 1.0]`，成功/共现频率的无偏估计 |
| `properties: Option<Json>` | 统计量载荷 | JSON，见 2.3 |
| `source_type` | 来源标记 | 固定 `"causal_observation"` |
| `source_id` | 证据溯源 | 触发该次观测的 `trajectory.id` |
| `knowledge_base_id` | 保留 | 空串（与现有 `upsert_relation` 行为一致） |

### 2.2 实体 ID 编码（URI scheme）

有意采用 URI 式字符串而非先建 `knowledge_entities` 记录——与现有 `upsert_relation(source_id, target_id, ...)` 的用法一致，且因果观测频繁、不需要实体元数据。

```
tool:<tool_name>           例：tool:read_file
outcome:<success|failure|partial|abandoned>
topic:<topic_slug>         例：topic:refactor_auth   （小写、空格转下划线、截断 64 字符）
intent:<PredictedIntent 变体名>
```

### 2.3 `properties` JSON 载荷

```json
{
  "confidence": 0.62,
  "observations": 14,
  "positive": 11,
  "delay_mean_ms": 1800.0,
  "delay_var_ms": 160000.0,
  "last_observed_at": 1789000000
}
```

| 键 | 类型 | 说明 |
|----|------|------|
| `confidence` | f64 | 置信度，`min(0.99, n/(n+3))`，n = observations |
| `observations` | u64 | 总观测次数 |
| `positive` | u64 | 命中次数（工具未报错 / 结果匹配） |
| `delay_mean_ms` | f64 | 时间延迟均值，Welford 在线更新 |
| `delay_var_ms` | f64 | 延迟方差（**存方差非标准差**，供 Welford 增量使用） |
| `last_observed_at` | i64 | 末次观测时间戳（epoch 秒） |

**向后兼容**：读取时所有字段用 `unwrap_or_default()` 兜底，缺失字段不报错。

### 2.4 三类因果边

| 类型 | 因 → 果 | 观测时机 | 主要消费者 |
|------|---------|---------|-----------|
| **T（工具序列）** | `tool:A` → `tool:B` | `save_trajectory` 时遍历相邻 step | `task_prefetcher`（预取时机） |
| **O（工具结果）** | `tool:A` → `outcome:success` | `save_trajectory` 时遍历 `tool_results` | 工具选择排序、失败归因 |
| **P（话题结果）** | `topic:X` → `outcome:success` | `save_trajectory` 末尾单条 | `suggestion_engine`、可解释性 |

---

## 三、核心算法

### 3.1 强度更新（无偏样本均值）

不使用 LAAP 的固定学习率，改用无偏估计：

```rust
// 新观测 o ∈ {0.0, 1.0}，历史 n 次观测、强度 w
let n_new = n + 1;
let w_new = (w * n as f64 + o) / n_new as f64;
// 等价于：w += (o - w) * (1.0 / n_new)
```

理由：工具成功率是相对稳定的量，样本均值比固定学习率更抗噪且收敛无偏。

### 3.2 置信度（Laplace 平滑）

```rust
let confidence = (n as f64 / (n as f64 + 3.0)).min(0.99);
```

n=3 → 0.50；n=6 → 0.67；n=15 → 0.83。低样本时自动抑制，避免小样本误导。

> 后续若需要更严格的下界保证，可升级为 Wilson score lower bound，接口不变。

### 3.3 延迟统计（Welford 在线算法）

```rust
// 增量更新均值与方差，避免保存全部历史样本
let delta = x - mean;
mean += delta / n as f64;
let delta2 = x - mean;
var += delta * delta2;   // 累积 M2，样本方差 = M2 / (n-1)
```

存储时直接存累积 M2（字段名 `delay_var_ms`），读取时除以 `(n-1)` 得样本方差。

### 3.4 因果链传播

BFS，深度 ≤ 5：

- 累计强度 = 路径上各边 `weight` 之积（独立假设）
- 累计延迟 = 路径上各边 `delay_mean_ms` 之和
- 剪枝：累计强度 < `min_confidence` 时停止扩展
- 防环：`visited: HashSet<String>`

---

## 四、分阶段实施

### P0 — 核心模块（无侵入，纯新增）

**新增文件**：`crates/trajectory/src/causal.rs`

| 元素 | 签名 |
|------|------|
| 载荷结构 | `pub struct CausalEdgeStats { confidence, observations, positive, delay_mean_ms, delay_m2_ms, last_observed_at }` |
| 链结构 | `pub struct CausalChain { path: Vec<String>, strength: f64, total_delay_ms: i64 }` |
| 观测 | `pub async fn observe_edge(db: &DatabaseConnection, cause: &str, effect: &str, positive: bool, delay_ms: Option<i64>, trajectory_id: &str) -> Result<CausalEdgeStats, TrajectoryError>` |
| 查询 | `pub async fn get_edge(db: &DatabaseConnection, cause: &str, effect: &str) -> Result<Option<CausalEdgeStats>, TrajectoryError>` |
| 链预测 | `pub async fn predict_chain(db: &DatabaseConnection, from: &str, max_depth: usize, min_confidence: f64) -> Result<Vec<CausalChain>, TrajectoryError>` |
| 轨迹抽取 | `pub async fn observe_from_trajectory(db: &DatabaseConnection, t: &Trajectory) -> Result<usize, TrajectoryError>` |

**关键实现要点**：

1. `observe_edge` 采用 **read-then-write**（先 `SELECT ... WHERE source_entity_id=? AND target_entity_id=? AND relation_type='causes'`，存在则 UPDATE，不存在则 INSERT）。**不使用** `on_conflict`——表上无唯一索引，且现有 `upsert_relation` 已因此失效。

2. 实体 ID 规范化：`normalize_topic()` 转小写、空格/斜杠转下划线、非字母数字剔除、截断 64 字符。

3. `properties` 读写统一走 `serde_json::Value`，缺失键用 `unwrap_or_default()`。

4. 全部日志用 `tracing::debug!`/`warn!`，不引入 `println!`。

**注册**：`crates/trajectory/src/lib.rs` 增加

```rust
mod causal;
pub use causal::{CausalChain, CausalEdgeStats, observe_edge, observe_from_trajectory, predict_chain};
```

**验收**：
- `cargo check -p axagent-trajectory` 通过
- `cargo clippy -p axagent-trajectory --all-targets -- -D warnings` 零警告
- `cargo fmt --check` 通过
- 单元测试覆盖：首次观测 / 重复观测收敛 / Welford 方差 / 链 BFS 剪枝 / 环防护 / 缺失字段兜底

---

### P1 — 埋点（最小侵入）

**改动文件**：`crates/trajectory/src/storage.rs`

1. `TrajectoryStorage` 新增字段 `causal_enabled: bool`（默认 `false`）
2. 新增 `pub fn set_causal_enabled(&mut self, enabled: bool)` 与 `pub fn is_causal_enabled(&self) -> bool`
3. 在 `save_trajectory`（`storage.rs:82`）末尾、事务提交**之后**插入：

```rust
if self.causal_enabled {
    match crate::causal::observe_from_trajectory(&self.db, t).await {
        Ok(n) => tracing::debug!("causal: observed {} edges from trajectory {}", n, t.id),
        Err(e) => tracing::warn!("causal: observation failed for {}: {}", t.id, e),
    }
}
```

**设计取舍**：观测失败只记 `warn!`，**绝不阻断主流程**。因果边是增强特性，不能影响轨迹持久化。

**`observe_from_trajectory` 抽取逻辑**：

```
对每个 step_i（含 tool_results）:
  T 边: 若 step_i 与 step_{i+1} 均有工具调用
        → observe_edge(tool:A, tool:B, positive=true,
                       delay = step_{i+1}.timestamp_ms - step_i.timestamp_ms)
  O 边: 对 step_i.tool_results 中每个 result
        → positive = !result.is_error
        → observe_edge(tool:name, outcome:<t.outcome>, positive, delay = 轨迹时长 - step_i.timestamp_ms)
轨迹末尾:
  P 边: observe_edge(topic:<normalized>, outcome:<t.outcome>,
                     positive = matches!(t.outcome, Success), delay = None)
```

**验收**：
- `cargo check -p axagent-trajectory` 通过
- 关闭状态下（默认）行为与改动前**完全一致**——用现有 `save_trajectory` 测试回归验证
- 集成测试：开启后保存一条含 3 步工具调用的轨迹，断言边数 ≥ 4 且 `properties.observations == 1`

---

### P2 — 消费点（让因果边真正被用起来）

#### P2.1 `task_prefetcher.rs` — 延迟驱动预取

现状：`TaskPrefetcher` 无 DB 依赖，`estimated_prepare_time_ms` 由 `estimate_completion_time()` 硬编码估算。

**改动**：

1. `TaskPrefetcher` 新增字段 `delay_hints: HashMap<String, i64>`
2. 新增方法：

```rust
/// 注入因果延迟提示。key 为 `format!("{}|{}", cause, effect)`
pub fn set_causal_delay_hints(&mut self, hints: HashMap<String, i64>)
```

3. 修改 `prefetch_for_prediction()`：优先用 `delay_hints` 覆盖 `estimated_prepare_time_ms`，未命中则回退原逻辑。

4. 新增 `PrefetcherConfig` 字段 `use_causal_hints: bool`（默认 `false`），保持 `Deserialize` 向后兼容（用 `#[serde(default)]`）。

**上层装配**（在有 DB 的调用侧，新增独立函数避免污染 prefetcher）：

```rust
// causal.rs
pub async fn build_prefetch_delay_hints(
    db: &DatabaseConnection,
    from: &str,
) -> Result<HashMap<String, i64>, TrajectoryError>
```

#### P2.2 `suggestion_engine.rs` — 因果依据建议

在 `generate_suggestions`（`suggestion_engine.rs:105`）中，当 `personalization` 开启且因果边可用时，追加一类基于 `predict_chain` 的建议，附带可解释文案：

> 「推荐 Edit：Read → Edit 置信度 0.78（观测 14 次），平均间隔 1.8 秒」

#### P2.3 `knowledge_graph_provider.rs` — 检索过滤 ✅ 已完成（2026-09-04）

**决策：因果边默认不参与 RAG 图检索**（因果边是行为统计，不是文档知识），仅由专用 API 消费。

实施时发现并修复两个既有问题：

1. **`kg_repo::graph_enhanced_search` 关系查询分组 bug**（原 `knowledge_graph.rs:681`）：
   链式 `kb_id.eq(kb).and(source_in).or(target_in)` 的实际语义是
   `(kb AND source_in) OR target_in` —— OR 分支丢掉 kb_id 约束，跨 KB 关系可泄露进检索。
   修复为 `Condition::all().add(kb).add(any(source_in, target_in)).add(relation_type != "causes")`，
   分组正确 + 显式排除因果边（双保险）。
2. **`harness/tests/serialization_contracts.rs`** 三处 `AgentResult` 字面量缺 `session_id`，
   长期阻塞 `--all-targets` 验收门禁，已补齐。

`CAUSAL_RELATION_TYPE = "causes"` 权威定义移至 `harness/src/knowledge_graph.rs`（契约层），
trajectory 侧 re-export，dao 检索方直接引用 harness 原件。

回归测试：`dao/tests/knowledge_graph_search.rs` ——
对抗样本为「同 KB 因果边」与「跨 KB 关系 target 撞 seed」，断言检索结果只含本 KB 文档关系。

**验收**：
- `cargo check --workspace`、`clippy --all-targets -- -D warnings` 零警告 ✅
- 预取时机测试：注入 hint 后 `estimated_prepare_time_ms` 等于 hint 值 ✅（P2.1）
- 建议生成测试：低置信度边（n<3）不产生建议 ✅（P2.2）

---

### P3 — 需求驱动的主动调度 → 已随 AgentRuntime 一并删除（2026-09-04 终局裁决）

**核查结论**：P3 的落点 `AgentRuntime` 经全项目引用核查确认为**从未接线的平行路径**——生产三条对话链路
（`commands/agent/mod.rs`、`commands/fleet/executor.rs`、`commands/plan.rs`）全部直接走
`create_conversation_runtime` + `SessionManager::run_turn_with_tools`。接线不可行：`AgentRuntime::run()`
是同步一次性调用（无流式、无取消 token），接入生产意味着流式管线倒退；其内部功能（人格注入、
reasoning 路由、proactive tick）要么生产已有独立实现（`commands/agent/mod.rs:627`、coordinator、
NudgeService），要么本身空转（NoopSlashCommandProcessor、仅打日志的 engine 选择）。

**裁决：删除**。级联删除（唯一消费者均为 AgentRuntime / agent_adapter）：

| 文件 | 行数 | 说明 |
|---|---|---|
| `agent/src/agent_runtime.rs` | 597 | AgentRuntime/AgentEvent/AgentOutput/AgentRuntimeError |
| `agent/src/agent_adapter.rs` | 674 | AgentImplAdapter/AgentRuntimeAdapter（唯一消费上述两类型处） |
| `agent/src/proactive_mode.rs` | 493 | ProactiveMode/ProactiveSignal（P3 门控随之移除） |
| `agent/src/slash_command.rs` | 104 | apply_slash_command_for_agent（Noop 处理器空转） |
| `reasoning_router.rs` 内 4 项 | ~90 | select_with_llm_hint/FEATURE_ANALYSIS_PROMPT/ab_test_route/create_ab_trial（零引用） |
| `noop_kit.rs` NoopSlashCommandProcessor | ~16 | 唯一消费者已删 |

**注**：生产侧的主动行为由 `NudgeService`（`proactive_nudge_enabled` 门控，commands/agent:2237 注入
prompt）承担。若未来需要 novelty/预测误差驱动的主动 tick，正确落点是 NudgeService 路径而非复活
AgentRuntime。P3 曾构建的信号门控逻辑（novelty/prediction_error 阈值）已在删除范围，其阈值设计
保留在本 git 历史与本节记录中。

**验收（删除后）**：cargo check --workspace 零警告 / clippy -p axagent-agent --all-targets -D warnings
零警告 / agent 全量测试 1966 lib + 集成全绿 / fmt 通过。

### P4 — 前端可视化与 i18n（可选）

若需暴露到 UI：

- 知识图谱视图中，因果边用不同样式（虚线 + 箭头粗细映射 `weight`）
- 悬停显示：`置信度 0.78 · 观测 14 次 · 平均延迟 1.8s`
- **i18n**：新增 key 必须同步 11 个语言包，且通过 `scripts/check-hardcoded-i18n.sh --strict`

---

## 五、测试方案

### 5.1 单元测试（`causal.rs` 内 `#[cfg(test)]`）

| 用例 | 断言 |
|------|------|
| `test_first_observation_creates_edge` | observations=1，confidence≈0.25，weight=1.0（positive） |
| `test_repeated_observation_converges` | 8 次 positive + 2 次 negative → weight≈0.8 |
| `test_confidence_grows_and_caps` | n=100 时 confidence ≤ 0.99 |
| `test_welford_variance` | 手工序列 [10,20,30] → mean=20, 样本方差=100 |
| `test_chain_bfs_pruning` | 累计强度低于阈值时不再扩展 |
| `test_chain_cycle_protection` | A→B→A 不无限循环 |
| `test_missing_properties_fallback` | properties 为 `{}` 时不 panic |
| `test_topic_normalization` | `"Refactor Auth/Login!!"` → `"refactor_auth_login"` |

### 5.2 集成测试（`crates/trajectory/tests/causal_integration.rs`）

使用 dev-dependency `axagent-dao` 的 `create_test_pool`（内存库 + 全量迁移）：

| 用例 | 断言 |
|------|------|
| `test_observe_from_trajectory_end_to_end` | 构造含 3 步工具调用的轨迹 → 边数 ≥ 4 |
| `test_edge_idempotent_update` | 同一条轨迹保存两次 → observations 累加而非新建行 |
| `test_predict_chain_reads_persisted` | 落库后 `predict_chain` 返回一致结果 |
| `test_disabled_by_default` | 默认 `causal_enabled=false` 时不产生任何边 |

### 5.3 回归

`save_trajectory` 在开关关闭时行为不变——由现有测试套件覆盖，必须全绿。

---

## 六、风险与回滚

| 风险 | 等级 | 缓解 |
|------|------|------|
| 因果边写入失败影响轨迹保存 | 高 | 观测在事务提交后执行，失败仅 `warn!`，不阻断 |
| `properties` 在 SQLite/PG 双端类型差异 | 中 | 统一走 `serde_json::Value`，集成测试跑 SQLite，CI 若覆盖 PG 需同步验证 |
| 因果边污染 RAG 图检索 | 中 | P2.3 明确默认排除；`relation_type_filters` 已支持 |
| 边数量无上限增长 | 中 | 工具名与话题名天然有限；额外加 `max_edges_per_trajectory`（建议 32）截断 |
| 无唯一索引导致并发重复行 | 低 | 写入频率低；若后续成为瓶颈，补 `v134` 部分唯一索引（需先去重） |
| 小样本误导决策 | 中 | `confidence = n/(n+3)` 天然抑制；消费侧强制 `min_confidence ≥ 0.35` |

**回滚**：P0 为纯新增文件，删除 `causal.rs` 与 `lib.rs` 两行注册即可。P1 改动受 `causal_enabled` 开关保护，设 `false` 即回到原行为，无需回滚代码。

---

## 七、验收清单

按项目 AGENTS.md 质量门禁，每个阶段提交前必须通过：

```bash
cargo fmt --all
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
npx tsc --noEmit          # 若涉及前端
npx dprint check          # 若涉及前端
bash scripts/check-hardcoded-i18n.sh --strict   # 若涉及 UI 文案
```

**禁止**：
- 用 `#[allow(dead_code)]` / `#[allow(unused)]` 绕过 lint——无用代码直接删除（P0 级铁律）
- 任何 git 操作（用户零接触铁律）

---

## 八、工作量与优先级

| 阶段 | 内容 | 新增/改动 | 优先级 |
|------|------|----------|--------|
| P0 | `causal.rs` 核心模块 | +1 文件，lib.rs +2 行 | **必须** |
| P1 | `storage.rs` 埋点 | ~15 行 | **必须** |
| P2 | 三个消费点 | ~80 行 | 高 |
| P3 | 需求驱动调度 | ~50 行 | 中（可独立） |
| P4 | 前端 + i18n | 视 UI 范围 | 低 |

**建议顺序**：P0 → P1 → P2.1（预收效最快）→ P2.2 → P2.3 → P3 → P4。

P0 + P1 即可让系统开始积累因果数据；P2.1 是第一个能看到实际效果的消费点。

---

## 九、附：为什么不用现有 `upsert_relation`

`dao/src/repo/knowledge_graph.rs:583` 的 `upsert_relation` 看似可用，实际有三处不满足：

```rust
let id = format!("rel_{}", gen_id());          // ① 每次新 id
let am = knowledge_relations::ActiveModel {
    knowledge_base_id: Set(String::new()),      // ② 无法指定 kb
    properties: Set(None),                      // ③ 无法写入统计量
    ...
};
knowledge_relations::Entity::insert(am)
    .on_conflict(
        OnConflict::column(knowledge_relations::Column::Id)  // ① 冲突键 = 新 id，永不冲突
        ...
    )
```

**① 是 bug**：`on_conflict` 用主键 `Id`，但 `Id` 是刚生成的唯一值，冲突条件永远不成立，函数实际退化为纯 INSERT。重复调用会产生重复行，且 `weight` 永远不会被更新。

本方案自建 read-then-write 逻辑，同时绕过 ② ③。该 bug 不在本方案修复范围，建议单独记录。
