# A / B / C 三组本体接线实施报告（2026-09-14）

> 状态：active ｜ 日期：2026-09-14 ｜ 对应版本：v2.9.7


> **裁定来源**：用户「**按照 A、B、C 顺序依次实施**」（对应 `AUDIT-ontology-vs-existing-graph-structures-2026-09-14.md` 的落点清单 A1/A2 → B1/B2 → C1）。
> **本次姿态**：不再产出「形态齐全但零消费」的新词汇表，而是**把仓库里已有的隐含约定提成可测公理，并接到真实调用链上**。
> 每条交付物末尾附**生产消费点统计**（上一轮的教训：806 行本体、生产引用 4 行）。

---

## 0. 一句话结论

| 组 | 动作 | 关键产出 | 生产消费 |
|---|---|---|---|
| **A** | 亲和度表外迁 + 解析失败可观测 | `page_type.rs` 成载体层本体（词汇表 **14** 变体 + 4 条公理 + 严格解析） | `compute_type_affinity` ← 相关度打分主路径；`unresolved_types` ← **UI 警示条**（A2-升级后新增消费方） |
| **B** | 图谱关系/实体类型词汇表 + 写入端接线 | `RELATION_DECLS` **18** 条（含域/值域）+ 前缀族 + **数据驱动列**第三类登记 + 校验器；`ENTITY_TYPE_DECLS` **11** 条 | 3 条常量 + 2 处校验（dao / 导入）+ **8 处** `GraphNodeType` |
| **C** | DAG 端口公理 | `workflow_port_axioms.rs` 6 类违规（**4 error / 2 warning**）、公理照抄引擎 | `validate_workflow_template`（UI 校验命令，`error` 级会**挡保存**） |
| **D** | 待裁决 5 项按推荐值实施 | 见 **§6**（D3 补变体 / D5 无迁移收敛 / B2 裁剪 / C1 分级+修真缺陷 / A2 进 UI） | 4 项有生产消费点；**1 个真实缺陷被修复**（`prod-startup-mvp` 的 Switch 分支全灭） |

---

## A 组：载体层本体（`PageType`）

### A1 亲和度公理外迁

**病灶**：`graph_dtos.rs` 原 `compute_type_affinity` 把一条**不相交兼容公理**
（「跨类型只允许 `Entity`↔`Concept`」）硬编码在 `match (&type_a, &type_b)` 函数体里。
它**在跑**（`total_score` 以权重 1.0 计入，影响 `get_top_k_related` 排序与 `graph_insights` 惊奇度），
但不可测、不可审计，也无法回答「仓库里一共有几套类型判定」。

**改动**（`crates/harness/src/page_type.rs`）：

| 声明 | 内容 |
|---|---|
| `ALL_PAGE_TYPES` | 8 个变体的全量清单（`as_str` 唯一性 + 往返由测试锁定） |
| 公理 **A1** `SAME_TYPE_AFFINITY` | `Entity/Concept = 1.0`、`SourceSummary = 0.5` |
| 公理 **A1′** `SAME_TYPE_FALLBACK` + `SAME_TYPE_FALLBACK_TYPES` | 兜底 `0.3`，**适用类型显式列出**（禁止 `_` 静默兜底） |
| 公理 **A2** `CROSS_TYPE_AFFINITY` | 唯一被允许的跨类型对 `Entity↔Concept = 0.5`，查询**双向对称** |
| 公理 **A2′** `CROSS_TYPE_FORBIDDEN = 0.0` | 不相交公理：其余跨类型对一律 0 |
| 值域公理 `AFFINITY_RANGE` | `[0,1]`（下游 `normalized_total` 除以常数 10.0，越界会让归一化分数越界） |
| `parse_strict` | 严格解析：未识别 ⇒ `None`（`FromStr` 保持历史宽松契约，两者一致性由测试锁定） |
| 纯校验器 3 个 | `same_type_table_ok` / `cross_type_table_ok` / **`vocabulary_coverage_is_total`**（覆盖制，新增变体漏登记即红） |

**为什么放在 `page_type.rs` 而不是 `domain_ontology.rs`**：`domain_ontology` 是**领域层**（行业/产业链/瓶颈三力），
本模块是**载体层**（页面形态），两者不互相包含。**本体不是一个大文件，每条词汇表自己的公理表就在它旁边** ——
塞进 `domain_ontology` 会造出第三处并列词汇表（这正是上一轮已犯过的错）。

### A2 解析失败可观测化

原实现：

```rust
self.nodes.get(node_id)
    .and_then(|n| n.node_type.parse::<PageType>().ok())
    .unwrap_or(PageType::Unknown)      // 「节点缺失」与「字面量不在词汇表」两种失败被吞成同一个值
```

**改动**：

- 新增 `PageTypeLookupError { MissingNode, UnrecognizedType { node_id, raw } }` —— 两种失败**可区分**；
- `get_page_type_strict()` 成为唯一实现体，`get_page_type()` 退化为「一层显式兜底」⇒
  「兜底发生在哪、兜成了什么」在源码里只有**一处**可看；
- `LinkGraph` 新增 `unresolved_types: Vec<(node_id, raw_type)>`（**按 id 排序**，确定性），
  由 `from_graph_data` **一次性**算出并 `tracing::warn!` 一次 —— 判定函数 `compute_type_affinity`
  处在 O(n²) 路径上，不能在那里打日志。

> **行为变更**：**零**（数值逐值等价，见下）；新增的只是一条 `warn` 日志。
> 刻意**不做** trim / 大小写归一 —— 那会改变现有行为（`"Entity"` 目前 ⇒ `Unknown`），属独立产品决策，不夹带。

**等价性锁**（`graph_dtos.rs::test_type_affinity_matches_legacy_hardcoded_table`）：
把**改造前的源码逐字复制**成 `legacy_affinity()`，在 10×10 类型对（8 个词汇表字面量 + 实测存在的
越界值 `log` / `daily`）上与新实现逐值比对 ⇒ 全等才通过。

### A 组真实数据发现（新，未擅自改）

`PageType` 的词汇表与**实际写入值**是两套：

| 值 | 谁写 | `PageType` 里有吗 |
|---|---|---|
| `log` | `wiki_compiler.rs:671 is_valid_page_type` **允许**（且 `page_type_dir("log") => ""`） | ❌ 无此变体 |
| `note` | `note.rs:543`（DB NULL 时的默认）、`page_type_dir`/测试都用 | ✅ 有，但 `is_valid_page_type` **拒绝**它（`wiki_compiler.rs:1551` 明确断言 `!is_valid_page_type("note")`） |
| `daily` / `knowledge` / `doc` / `knowledge_document` / `synced` | `commands/wiki.rs:1637/1883`、`commands/knowledge_source.rs:237`、`init/opc_knowledge.rs:254`、`init/services.rs:375` | ❌ 全部无变体 |

**后果（可计算）**：这些页面进图谱后 `node_type` 解析失败 ⇒ 亲和度静默落 `0.3`（同类型）/ `0.0`（跨类型）；
且在 `graph_insights.rs:136` 被算作 **「跨类型连接」惊奇度 `+1.5`** —— 与真实的跨类型连接**无法区分**。

⇒ 已登记为 **D3**（待裁决，见 §6）。本轮**只让降级可见**（warn + `unresolved_types`），未改判定语义。

### A 组生产消费点

| 符号 | 消费 |
|---|---|
| `same_type_affinity` / `cross_type_affinity` | `graph_dtos::compute_type_affinity`（**生产、热路径**）← `compute_relevance` ← `get_top_k_related` / `build_relevance_edges` ← `agent/graph_insights` |
| `parse_strict` | `graph_dtos::get_page_type_strict` + `from_graph_data`（2 处生产） |
| `unresolved_types()` / `get_page_type_strict()` | ⚠ **外部调用方 0** —— 降级信号由 `from_graph_data` 的 `warn` 发出；两个查询 API 当时只有测试在用 |
| `collect_unresolved_type_stats`（第二轮新增） | **有真实生产消费方**：`get_wiki_graph_cached` 融合后回填 → `GraphData.unresolved_types` → `WikiGraphPage` 警示条（见 §6.5）。原「查询 API 零消费」的遗留**已消除一半** |

---

## B 组：图谱关系 / 实体类型词汇表

### B1 关系词汇表（含定义域·值域）

**病灶**：`knowledge_relations.relation_type` 是自由文本 `String`，全仓取值来自 **5 条互不相干的来源**，
**没有任何一处声明 domain/range** ⇒ 可写入 `(Company)-[causes]->(Note)` 而无人报错。

**改动**（`crates/harness/src/knowledge_graph.rs`）：

- `RELATION_DECLS` **19 条**（**第二轮 D5 收敛后为 18 条** —— 11 个 trajectory 变体改 `Literal`、`contains` 两条合并为一条，见 §6.2），每条带 `含义 / evidence(文件:行) / observed`；
- **前缀族** `RELATION_FAMILIES`：`employ_*`（`format!("employ_{position}")` ⇒ 职位有多少词表就有多少项，只能登记族）；
- `ENTITY_TYPE_DECLS` **7 条**（**第二轮由真实数据修正为 11 条**，见 §6.2），（`entity_type` 是**开放词表**，只登记、不闭合）；
- `GraphTypeViolation` 4 类 + 校验入口 `validate_relation_id` / `validate_relation_write`（含域/值域）/ `validate_entity_type`；
- 纯校验器 3 个（`relation_table_ok` / `relation_family_table_ok` / `entity_type_table_ok`）。

**两条刻意的纪律（写进模块文档）**：

1. **不发明约束**：只登记**观察到**的域/值域。写入方本身拿不到节点类的关系（`upsert_relation(source_id, target_id, relation_type, weight)` 只有 id），
   其 `domain`/`range` 记为 `None` = **宽松**，而不是猜一个。测试 `test_domain_range_enforced_only_where_declared`
   的反向用例正是「未声明域/值域的关系**不得**报错」。
2. **只登记、不硬拦**：`edges.csv` 的 `rtype` 直接来自**数据文件**（`commands/knowledge.rs:1725`），
   闭合词表必然误伤导入 ⇒ 校验产出违规清单，由调用方决定 `error` / `warning`。

**重大发现：同一列存在两套编码**

| 编码 | 写入方 | DB 里实际长什么样 |
|---|---|---|
| `Literal` | `trajectory/src/causal.rs:305`、`dao/src/repo/conversation.rs:716/735` | `causes` / `contains` / `follows` |
| `JsonEncodedVariant` | `trajectory/src/storage.rs:911`（`serde_json::to_string`） | **带引号的 `"part_of"`** |

该路径**自洽**（读方 `storage.rs:1769` 用 `serde_json::from_str(&format!("\"{}\"", …))` 反解，实测确认），
但与裸字面量路径**不互通**：`Column::RelationType.eq("part_of")` **匹配不到**该行。
⇒ 任何按字面量过滤的读方（`knowledge_graph_provider.rs:110` 的 `relation_type_filters`、
`dao/src/repo/knowledge_graph.rs:716` 排除因果边）对本路径写入的行**全部失效**。

> 已登记为 **D5**。**第二轮已按推荐值实施**：**写端收敛 + 读端永久宽容 = 零迁移**（实测带引号行 = 0）—— 见 §6.2。
> （第一轮差点改错的：`serde_json::to_string` 的引号看着像 bug，读方却依赖它。）

**`contains` 同 id 两条登记**：裸字面量写的是「会话容器关系」，JSON 编码写的是「记忆实体图关系」，
是两条独立链路。故当时唯一键是 `(id, encoding)`，且 `relation_decl()`（取第一条）之外必须提供
`relation_decls()`（取全部）—— 否则调用方会在不知情的情况下漏掉另一条链路。

> **第二轮修正（重要）**：收敛后两条链路在 DB 里是**同一个字面量** ⇒ 合并为 1 条登记（两条出处都留在 `evidence`），
> 且 **唯一键必须收紧为 `id`** —— 否则「再引入第二套编码」时表能过、而 `relation_decl` 只返回第一条
> ⇒ 另一条链路静默消失，**收敛可被悄悄逆转**。见 §6.2 与 `relation_table_ok` 的文档。

### B1 写入端接线（5 处）

| 位置 | 动作 |
|---|---|
| `dao/src/repo/conversation.rs:720` | 裸字面量 `"contains"` → `RELATION_CONTAINS` |
| `dao/src/repo/conversation.rs:739` | 裸字面量 `"follows"` → `RELATION_FOLLOWS` |
| `dao/src/repo/knowledge_graph.rs:605` | `upsert_relation` 写入前 `validate_relation_id` + `warn_on_violations`（**不阻断**） |
| `src/commands/knowledge.rs:1805` | `graph_import` 导入前校验，**按「类型」去重上报**（避免逐行刷屏） |
| `src/commands/knowledge_graph.rs:305` | 裸字面量 `"mentions"` → `RELATION_MENTIONS` |

### B2 v113 死词汇表：把「声明未接线」改成「有接线 + 显式缺口」

| 对象 | 改造前 | 改造后 |
|---|---|---|
| `GraphNodeType` / `as_str()` | **定义外 0 引用**、`as_str()` **0 次调用** | **8 处** `node_type` 写入点由硬编码 `String::from("entity")` 改走 `GraphNodeType::Entity.as_str()` ⇒ 枚举从「零消费」变为「有消费」 |
| `CreateMultiSourceEntityInput` | 文档称「仅定义 + trait 签名，无实现」（**我上一轮的判断有误**） | 实际**有默认实现**，但它**丢弃** `source_type` / `source_id` / `node_type` / `external_id` / `aliases` ⇒ 已补写文档说明 + 走默认实现时打 `warn` |
| `GraphSourceType`（4 值） | 0 外部引用 | **第二轮已删除**（连 `CreateMultiSourceEntityInput` 与 3 个 trait 默认方法一并裁掉）—— 见 §6.3 |
| `memory_item` / `obsidian_note` | 全仓仅出现在定义处 | 同上 |

> **自我纠错**：上一份报告写「`CreateMultiSourceEntityInput` 仅定义 + trait 签名、无实现」——**错了**。
> 它有默认实现，真正的病是「默认实现把多源字段悄悄丢掉」——比「无实现」更难发现。

### B 组生产消费点

| 符号 | 消费点 |
|---|---|
| `RELATION_CONTAINS` / `RELATION_FOLLOWS` / `RELATION_MENTIONS` | 3（1 / 1 / 1） |
| `validate_relation_id` + `warn_on_violations` | 2 + 2（dao `upsert_relation`、commands 导入） |
| `RELATION_DECLS` / `RELATION_FAMILIES` | **传递消费**：`validate_relation_id` → `is_declared_relation` → `relation_decl` / `relation_family_of` ⇒ 词表本身在跑 |
| `GraphNodeType` | **8** |

---

## C 组：DAG 端口公理

**病灶**：`WorkflowEdge.source_handle: Option<String>` 无类型；它与 `Switch` case label / `Condition` 分支的
对应关系**只写在注释里**（`seed_stock_analysis.rs:3548` 就是一段这样的注释），
而 `validate_workflow_template` 当时只查了「边的源/目标节点是否存在」与「`ParallelBranch` 是否源自 `Parallel`」。

**权威照抄引擎，不自行发明**：`rt-workflow/.../dag_store.rs`
- `:100-121` `switch_edge_should_follow`（4 处调度路径共用）
- `:188` / `:268` / `:446` / `:502` / `:690` —— 五处相同的兜底映射
  `source_handle` 缺省时 `ConditionTrue → "true"`、`ConditionFalse → "false"`

**公理与违规类型**（`crates/harness/src/workflow_port_axioms.rs`，662 行）：

| 违规 | 判据 |
|---|---|
| `handle_contradicts_edge_type` | `source_handle` 与 `edge_type` 推出的兜底分支**不一致** ⇒ 同一条边两个答案 |
| `condition_handle_not_a_branch` | `Condition` 出边 handle 既非 `true` 也非 `false` ⇒ 两个分支都不激活（死边） |
| `switch_edge_handle_not_a_case` | 具名出边的 handle 不对应任何 `case.label` ⇒ 永不激活（死边） |
| `switch_case_unreachable` | 某 `case.label` 没有出边（且未被默认分支覆盖）⇒ 该分支不可达 |
| `switch_default_double_activation` | `default_case` **同时**是具名边的 handle，且还存在默认（无 handle）出边 ⇒ 匹配到该值时**两条边同时激活** |
| `switch_default_branch_unreachable` | 声明了 `default_case` 却既无默认出边、也无 handle==该值的具名边 |

> `switch_default_double_activation` 是 2026-09-13 那次修复的**另一半**：
> 那次修的是「**未**声明 `default_case` ⇒ 默认退化成并行」；这次把「**声明了** `default_case` 且它同时是具名分支」这一半也纳入判据。

**接线**：`src/commands/workflow_template.rs:971` —— 违规进 `ValidationResult.warnings`
（`warning_type = v.kind()`，`node_id` 回填）。

> **为什么是 warning 不是 error**：`is_valid = errors.is_empty()`，升为 error 会让存量违规模板
> 在保存时被挡下 —— 那是产品决策。前端对 `warningType` 是**不透明字符串**（`workflow.types.ts:792`
> 声明为 `string`，无 switch / 无 i18n key 映射）⇒ 新增类型**不需要**补 11 语言 key（已核）。

**真实数据核对**（本轮实测，非声称）：

| 对象 | 结果 |
|---|---|
| `edge_cond(...)` 全部调用点（**约 160 条**，散在 30 个 seed 文件） | handle 与 `edge_type` **全部一致**（`"true"`↔`ConditionTrue`、`"false"`↔`ConditionFalse`）⇒ 公理 C1 **零违规** |
| 声明 `default_case` 的 Switch（全仓共 **2 个**：`s-gonogo`、`quality-gate`） | 两者均**不构成**双重激活（`s-gonogo` 无默认边；`quality-gate` 的 `default_case` 不是 case label） |
| 其余 Switch（未声明 `default_case`） | **未逐一核对** —— 判据已接线，后续由 `validate_workflow_template` 覆盖（如实说明覆盖边界） |

⇒ **C 组没有发现存量缺陷**，价值在于把一条只存在于注释里的约定变成了机器判据（防未来回归）。

### C 组生产消费点

| 符号 | 消费点 |
|---|---|
| `validate_port_axioms` | **1**：`validate_workflow_template`（Tauri 命令，UI「校验模板」入口） |
| `PortAxiomViolation::kind()` / `describe()` / `node_id()` | 同上（消息文本与公理同处，避免调用方各写一份） |

---

## 4. 验证

全部命令在**最终源码状态**下运行，原始输出留存于 `output/final*.txt`（第一轮）与 `output/final-r12-*.txt` / `output/v2-*.txt`（第二轮），供复核。
第二轮见下方「第二轮验证」表 —— **权限在于「退出码」而不是「日志里没报错」**，故一律用重定向取 `$?`。

| # | 命令 | 结果 |
|---|---|---|
| 1 | `cargo fmt --all -- --check` | ⚠ 全仓 1 处 diff，位于 `crates/rt-workflow/tests/rhai_syntax_check.rs`（**并发会话的文件，不在本次改动清单**）⇒ 对本次 12 个改动文件单独跑 `rustfmt --edition 2024 --check` → **EXIT=0** |
| 2 | `cargo check -p axagent-dao --all-targets` | **EXIT=0**（`Finished in 1m 32s`） |
| 3 | `cargo check -p axagent --lib` | **EXIT=0**（`Finished in 11m 30s`）—— 含 `workflow_template.rs:971` 的 C1 接线 |
| 4 | `cargo test -p axagent-harness --lib` | **EXIT=0｜664 passed / 0 failed**（首次 `final5`；**最终源码态复跑 `final6` 同为 664 passed / 0 failed**） |
| 5 | `cargo clippy -p axagent-harness -p axagent-dao -p axagent-trajectory --all-targets -- -D warnings` | **EXIT=0**（首次 `final4` 16m 06s；**最终源码态复跑 `final7` 1m 46s**）—— 三个 crate **零警告**；`--all-targets` ⇒ 测试代码同样在管辖内 |

> **为什么跑了两轮**：第 4/5 项首次跑完后，我又做了两处**纯字符串内容**修改（`describe()` 改英文、11 处 `evidence` 行号修正），
> 因此按纪律在**最终源码态**复跑（`final6`/`final7`）。
> **覆盖证明落在产物时间戳上**（不看 `Finished` 行，见判据 #111/#112）：
> 最后源码 mtime = **06:33:50**（`harness/src/knowledge_graph.rs`）；
> 而 `target/debug/.fingerprint/axagent-harness-*/test-lib-axagent_harness` = **06:49:47 / 06:49:02**、
> `lib-axagent_harness` = **06:49:45**、测试二进制 `deps/axagent_harness-2e331ee45a1e0428.exe` = **06:49**
> —— **全部晚于源码 mtime** ⇒ 两轮确实编译自最终源码，不存在「编旧源码得绿」。
| 6 | `node scripts/check-ontology-consistency.mjs` | ✅ 副本与权威源一致（口径分歧 2 项为既有 D1/D2） |
| 7 | `node scripts/type-consistency.mjs` | **0 errors**（434 warning 为存量） |
| 8 | `python scripts/check_i18n.py --strict` | **🎉 全部通过**（11 locale 语法 + zh-CN 无空值 + 代码引用 key 全有定义） |
| 9 | `node scripts/check-layer-discipline.mjs` | ❌ 3 处超基线，**全部落在并发会话的 `stock_workflow/sim_hook.rs` / `rhai_pm.rs`**（非本次改动）⇒ 本次未新增越界 |
| 10 | `npx dprint check` | `dprint.json` 只挂 ts/json/markdown/toml 四个插件（无 Rust）且 `.gitignore:50` 有 `docs/` ⇒ `output-file-paths` 在 `docs/` 下找到 **0** 文件。审计报告与 `.rs` 均不在其管辖内 |

### 第二轮验证（待裁决 5 项实施后，**在冻结源码上一次跑完**）

全部命令用**重定向**取退出码（`cmd > f 2>&1; echo $?`）—— **不接管道**：
`cmd | tail` 后的 `$?` 是 `tail` 的状态（恒 0），会把失败读成成功（本轮新增纪律 #136）。

| # | 命令 | 结果 | 原始输出 |
|---|---|---|---|
| F1 | `cargo test -p axagent-harness -p axagent-trajectory --lib` | **EXIT=0**｜harness **671 passed / 0 failed**、trajectory **421 passed / 0 failed**（均 `0 ignored`，无 `panicked` / `^error` 行） | `output/final-r12-test.txt` |
| F2 | `cargo run -p axagent-harness --example port_axiom_audit -- output/tpl-dump.json` | **EXIT=1**（**期望如此**）：**3** 处 error，全在 `prod-startup-mvp`；111 模板 / 956 节点 / 0 解析失败 | `output/final-r12-audit-real.txt` |
| F3 | 同上，`output/tpl-dump-patched.json`（打上种子修复） | **EXIT=0**：**0 违规**（6 类公理全部零命中） ⇒ 修复**充分**且不误伤其余 110 个模板 | `output/final-r12-audit-patched.txt` |
| F4 | `cargo check -p axagent --lib` | **EXIT=0**（`Finished in 11m 09s`）—— 含 `workflow_template.rs` 的 severity 分流接线 | `output/final-r12-root.txt` |
| F5 | `cargo clippy -p axagent-harness -p axagent-dao -p axagent-trajectory -p axagent-agent --all-targets -- -D warnings` | **EXIT=0**（`Finished in 1m 59s`）—— 四个 crate **零警告**；`--all-targets` ⇒ 测试代码同样在管辖内 | `output/v2-clippy.txt` |
| F6 | 改动文件逐个 `rustfmt --edition 2024 --check`（**14 个**） | 首次 **2 个 diff**（`knowledge_graph.rs` 3 处、`graph_dtos.rs` 1 处，均为空白归一）⇒ 已就地 `rustfmt`，复跑 **14/14 FMT-OK** | — |
| F7 | `npm run typecheck` | **EXIT=0**（前端含 `GraphView.tsx` / `WikiGraphPage.tsx` 的类型新增） | `output/v2-tsc.txt` |
| F8 | `python scripts/check_i18n.py --strict` | **🎉 全部通过**；另加一轮 **11 locale 逐份核对**新增 key 与插值变量 ⇒ `PARITY-OK` | — |
| F9 | `node scripts/check-i18n-untranslated.mjs` | **EXIT=0**（未翻译 0 项、新增 0 项） | `output/v2-i18n-untr.txt` |
| F10 | `bash scripts/check-hardcoded-i18n.sh --strict` | **EXIT=0**（Rule 1/2/3 与两条 strict 全部 PASS） | `output/v2-i18n-hard.txt` |
| F11 | `node scripts/type-consistency.mjs` | **EXIT=0**｜727 checks / **0 errors** / 434 warning（存量） | `output/v2-types.txt` |
| F12 | `node scripts/check-ontology-consistency.mjs` | **EXIT=0**｜副本与权威源一致；口径分歧仍为既有 D1/D2 两项 | `output/v2-ontology.txt` |
| F13 | `node scripts/check-layer-discipline.mjs` | ❌ **EXIT=1**：3 处超基线，**全部落在并发会话的 `stock_workflow/sim_hook.rs` / `rhai_pm.rs`**（与第一轮同一批，非本次改动）⇒ 本次**未新增**越界 | `output/v2-layers.txt` |

> **覆盖证明（不只看 `Finished` 行，见判据 #111/#112）**：
> 最终源码 mtime 最晚 = **07:33:26**（`graph_dtos.rs`；`knowledge_graph.rs` = `07:33:25`，来自 rustfmt）；
> 而测试二进制 `deps/axagent_harness-050bd14d3bc8ff56.exe` = **07:42:43**、
> `deps/axagent_trajectory-1e5b26af06312836.exe` = **07:44:08**，
> `target/debug/.fingerprint/axagent-harness-*` = **07:45:48 ~ 07:45:49**、
> `axagent-trajectory-*` = **07:46:31 ~ 07:47:02** —— **全部晚于源码 mtime**
> ⇒ F1~F5 确实编译自最终源码，不存在「编旧源码得绿」。

> **⚠ 一次自造的验证返工（已写入纪律 #137）**：F1 与 F5 的首轮（`v2-test.txt` / `v2-clippy.txt`）跑完后，
> 我才对两个文件执行 `rustfmt`（仅空白）—— **mtime 漂移使那一批的「编译自最终源码」证据作废**，
> 故在冻结源码后整批复跑（F1~F5）。**正确顺序：改码 → 格式化 → 跑验证（只跑一次）。**
> 不过首轮也留下了有价值的中间证据：它给出的 harness 计数 **671** 与本轮一致，
> 且 trajectory 计数由 419 → **421** 的增量正好等于新增的 2 个 `#[test]`（算术闭合）。

### 新测试的执行证据（按模块计数）

**第一轮**（原始计数）：

| 模块 | 用例数 | 说明 |
|---|---|---|
| `page_type::tests` | 11 | 含 `test_type_affinity_values_are_locked`、`test_vocabulary_coverage_is_total`、`test_parse_strict_rejects_out_of_vocabulary` |
| `graph_dtos::tests` | 4 | 含 `test_type_affinity_matches_legacy_hardcoded_table`（10×10 等价性锁）、`test_unresolved_types_reported_sorted` |
| `knowledge_graph::type_vocabulary_tests` | 13 | 含 `test_all_trajectory_variants_are_declared`、`test_ambiguous_ids_expose_all_declarations`、`test_table_validators_have_negative_control`、`test_dangling_contracts_are_exactly_the_documented_ones` |
| `workflow_port_axioms::tests` | 15 | 含 `test_branch_of_edge_type_mirrors_engine`、`test_table_validators_have_negative_control` |
| **第一轮合计新增** | **43** | 均 `... ok` |

**第二轮**（改造后计数，源码静态统计 = 实测执行计数）：

| 模块 | 改造前 → 后 | 新增/改造的用例 |
|---|---|---|
| `page_type::tests` | 11 → **12** | 新增 `test_observed_write_values_are_in_vocabulary`（锁 7 个写入值必须在词汇表内且不得只映射成 `Unknown` + 反向对照）；改造 `test_parse_strict_rejects_out_of_vocabulary`（`log`/`daily` 升格为合法值后，越界样本换成 `wiki_page` / `Entity ` / `""`） |
| `graph_dtos::tests` | 4 → **4** | 等价性锁矩阵改用 `ALL_PAGE_TYPES`（14 项）+ 2 个真越界值；`test_unresolved_types_reported_sorted` 输入换成 `wiki_page` / `not_a_page_type` |
| `knowledge_graph::type_vocabulary_tests` | 13 → **17** | 新增 4：`test_relation_ids_are_unique`、`test_merged_declaration_keeps_every_chain_evidence`、`test_no_observed_decl_uses_the_legacy_encoding`、`test_data_driven_column_values_are_shape_gated`、`test_case_inconsistent_entity_types_are_registered_and_visible`；删除/重写 3：`test_contains_exists_in_both_encodings`（→ 合并后的出处可见性锁）、`test_lookup_accepts_both_encodings`（→ `test_lookup_tolerates_legacy_quoted_form`）、`test_ambiguous_ids_expose_all_declarations`（→ `test_relation_decls_returns_exactly_one_per_id`） |
| `workflow_port_axioms::tests` | 15 → **17** | 新增 `test_severity_split_is_locked_by_engine_semantics`（6 类逐条登记 + 长度断言 + 未登记即 panic + 两侧非空反向对照）、`test_switch_named_default_handle_is_not_a_dead_edge`（handle == `default_case` 不报 + 负向对照 `go-live` 必报） |
| `trajectory::storage::tests` | 2(tokio) → **2 + 2** | 新增 2 个纯函数测试：`parse_stored_relation_type_tolerates_both_historical_encodings`（含 4 组形态边界 + 中文数据驱动值）、`strip_json_quotes_never_slices_inside_a_multibyte_char` |

**执行计数的独立佐证（算术闭合）**：

| crate | 第一轮实测 | 本轮新增 | 本轮实测 | 是否闭合 |
|---|---|---|---|---|
| `axagent-harness` | 664 passed | +1（page_type）+4（knowledge_graph）+2（workflow_port_axioms）= **+7** | **671 passed / 0 failed** | ✅ 664+7 = 671 |
| `axagent-trajectory` | 419 passed | +2（storage 纯函数） | **421 passed / 0 failed** | ✅ 419+2 = 421 |

（两 crate 均 `0 failed / 0 ignored`；无 `panicked` / `^error` 行。）
「源码静态统计 = 执行计数」这一闭合本身就是证据：**说明新写的测试真的被编译执行了**，
而不是「改了测试但没跑」。

> **注意判据 #118 的实证**：第一轮 `final1`（第一次 `cargo test -p axagent-harness --lib`）以 `EXIT=101` 失败 ——
> `error[E0507] EdgeType 未派生 Copy`，出在**测试**里（签名改取引用后测试调用点仍传值）。
> 而在同一时刻 `cargo check -p axagent-dao --all-targets` 是**绿的** —— 因为它只编译 harness 的 **lib**，
> 根本不进 `cfg(test)`。**下游 crate 的 check 绿 ≠ 你的 crate 编得过。**


### 证据引用自校验（收尾，同日第三轮）

`RELATION_DECLS` / `ENTITY_TYPE_DECLS` / `DATA_DRIVEN_COLUMN` 的每条 `evidence` 都带「文件:行」，
这类引用**会随任何上游编辑的行号漂移而腐烂**。第一轮修过一轮（历史记录在最末），但**修完仍不对**：

- 第一轮的做法是「报 ✅ 就交差」——脚本只验「行号存在」（30 个引用全部可定位），
  内容靠人工逐条判读，改完后**没有任何回归装置**；
- 第三轮用重写后的校验器（`scripts/check-decl-evidence.mjs`）一跑，**又抓到 4 条仍指错行**。

#### A. 拿到的 4 条腐烂（**已修**）

| 声明 | 原引用 | 该行实际内容 | 真出处 |
|---|---|---|---|
| `uses` | `src/commands/knowledge_graph.rs:615` | `assert_eq!(relations[0].relation_type, "mentions")` | `:575`（`:563` 是 fixture 输入） |
| `has_chairman` | `harness/src/knowledge_graph.rs:317` | `/// 定义域（源节点允许的节点类）…` | `:195`（`e.g., ["in_industry", "has_chairman"]`） |
| `in_industry` | 同上的 `:317`（**写在一个括注里**） | 同上 | `:195` |
| `DATA_DRIVEN_COLUMN` | `src/commands/knowledge.rs:1725` | `continue;` | `:1723`（`let rtype = fields[2]…`） |

> 成因**未逐一溯源**（不碰 git ⇒ 拿不到历史 diff）：既有本轮 B2 删除（`GraphSourceType` /
> `CreateMultiSourceEntityInput` / 3 个 trait 默认方法，均在本文件内）造成的整体上移，
> 也有第一轮遗留。可确定的是：**第一轮「已按行号逐条修正」的结论不成立**。

#### B. 元缺陷：**测试把错误行号锁死了**

```rust
// 原断言（错）—— 把 `1725` 当期望值
assert!(DATA_DRIVEN_COLUMN.evidence.contains("1725"), "数据驱动路径的出处必须写清楚");
```

它把**错的**行号写进了期望值 ⇒ 引用腐烂时测试照样绿，缺陷被测试本身掩盖。
改为「按 `evidence` 里的行号**去磁盘读那一行**，断言它真的在讲 `rtype`」（`include_str!` 读
`src/commands/knowledge.rs`），并做了**负对照**证明它真会红：

| 实验 | 结果 |
|---|---|
| evidence 指 `:1723`（正确） | `test result: ok. 671 passed; 0 failed` |
| evidence **故意**指 `:1719`（`continue;`） | `FAILED. 0 passed; 1 failed`，报错自证：`第 1719 行实为：continue;` |

#### C. 校验器重写的三处（旧版 `output/verify-evidence.mjs` 已删）

1. **不再把「非 `文件:行` 证据」当失败** —— 旧版把 6 条 DB 分布 / CSV 行数证据永远报「未能定位」
   ⇒ **恒定假红**（`EXIT=1` 永远成立）。现分三类：`LOCATED` / `NONLOC`（本来就不含文件:行）/
   `BROKEN`（文件不存在、行号越界、**该行是空行**）。只有 `BROKEN` 算失败。
2. **扫一段里的全部引用** —— 旧版只取首个匹配 ⇒ `in_industry` 括注里那条 `harness/…:317`
   **被静默漏扫**，正是它藏住了最后一条腐烂（判据 #7「0 命中 ≠ 没问题」的同型病）。
   顺带支持裸 `:行号`（承接前一个文件名）。
3. **补「内容与 id 同源」软判据** —— 结构校验抓不到「行号没越界、但指向完全无关的一行」，
   `uses` 指向一句 `"mentions"` 断言就是这样被抓出来的。白名单 2 条，均为人工判读后登记的
   **合法**例外（`causes` 的写入端用常量 `CAUSAL_RELATION_TYPE`，同义不同名；数据驱动列的 id 是
   DB 列名而出处在讲 CSV 的 `rtype` 列）。

#### D. 结果

```
主流程：声明块 30｜证据条目 35｜可定位 29｜腐烂 0｜内容存疑 0     ⇒ EXIT=0
--selftest：8 例 / 4 类样本正负对照（含 2 例「漏扫」回归）        ⇒ EXIT=0
退出码分级：0 全好｜1 引用腐烂（真失败）｜3 只有「内容存疑」（需人判）
```

#### E. 已接 CI（2026-09-14 第四轮，按指令「挂进 CI」）

按上一轮自己给的方案（**只拦结构腐烂**）接入两处：`scripts/ci-check.mjs`（本地镜像）
与 `.github/workflows/ci.yml`（真实 CI）。两处的命令行**逐字同源** —— 这是 `ci-check.mjs`
里写明的硬约束，不同源就会出现「本地绿、CI 红」的假安全感。

| 命令 | 用途 |
|---|---|
| `node scripts/check-decl-evidence.mjs --selftest` | 先跑正负对照：扫描器自身会撒谎，自检不过则后面的「通过」无意义 |
| `node scripts/check-decl-evidence.mjs --ci` | 正式门禁：只拦结构腐烂（文件不存在 / 行号越界 / 指到空行） |

**为什么新增 `--ci` 开关而不是裸跑**：脚本本就分两类判据 ——「结构腐烂」（客观错，修法唯一）
与「内容与 id 不同源」（软判据，会被「同一件事换了措辞」误伤，现有 2 条白名单即此类）。
裸跑会因软判据抖动而红，而它的修法是**改判据**而非修缺陷 ⇒ 红久了必被绕过。
`--ci` 只把后者降级为「打印到 stderr、退出 0」，前者照旧退出 1。

**负对照（证明「分流」是真的，而不是「两种模式恰好都绿」）**：

| 构造 | 不带 `--ci` | 带 `--ci` |
|---|---|---|
| `has_concept` 出处指向 `knowledge.rs:1750`（**存在、非空、但不含该 id**）⇒ 软判据命中 | **EXIT=3** | **EXIT=0** |
| 再指向 `knowledge.rs:99999`（行号越界）⇒ 结构腐烂 | — | **EXIT=1**（硬拦不放水） |
| 还原后 | EXIT=0 | EXIT=0 |

还原精度自证：与备份逐字节比对 `RESTORED_IDENTICAL=true`（负对照样本未残留）。

**接线自校验**（`output/verify-ci-wiring.mjs`，一次性的）：`node --check` 双绿；用 `yaml` 解析
`ci.yml` 得 7 个 job，`frontend-check` 里该步骤的 `run` 与目标命令**逐字相等**；
两侧命令集**本次未新增不一致**。判据刻意定为「相对基线不新增」而非「全量集合相等」——
两个文件**本来就**有结构性差异，拿绝对值当判据会把这次干净改动判成失败（判据过宽 = 假红）。

**验证边界（如实登记，勿读成「ci-check 全绿跑过」）**：本地镜像
`node scripts/ci-check.mjs --frontend-only` **跑不完** —— 它在**第 4 步**（分层护栏）因棘轮
超基线 3 处而 `process.exit(1)` 中断，那 3 处全在**并发会话正在改的**
`src-tauri/src/commands/stock_workflow/sim_hook.rs`（`commands-no-direct-db` 3 处）与
`rhai_pm.rs`（`commands-no-sibling-call` 1 处），与本次改动无关（同源项上一轮 F13 已登记）。
故新步骤的「会执行且会通过」不是靠跑一遍 ci-check 证明的，而是三件事共同证明：
① 语法 `node --check` 双绿；② 两条命令单独执行 `EXIT=0`；
③ 静态位置断言（`verify-ci-wiring.mjs` 第 4 段：位于 `canRunFrontend` 块内、
既有静态门禁之后、`--rust-only` 会跳过）。

> ✅ **顺带发现的既有漂移 —— 已修复（2026-09-14 当日第二案）**：
> ① 本地镜像缺 3 条 CI 已跑的门禁 —— `check-id-validation.mjs` / `check-errorcode-alignment.mjs`
> （`ci.yml:164/167`）、`check-rust-raw-map-err.mjs`（`ci.yml:230`）⇒ **本地会绿而 CI 会红**；
> ② 反向缺一条：`check-ngram-consistency.mjs` 只在本地镜像里。
> 另有 2 条是**假阳性**：`i18n-scan.mjs` 由 `check-hardcoded-i18n.sh:11`
> （`exec node scripts/i18n-scan.mjs "$@"`）包装调用；`ci-check.mjs` 只是文件头的用法注释。
>
> **修法与证据 → `AUDIT-ci-drift-skill-desc-2026-09-14.md`**（本地补 3 条；CI 补 ngram 一条，
> 含脚本 env 通道改造 + 「配置缺失」诚实 SKIP + 防静默 SKIP 的前置守卫；双向差集归零）。

> **第一轮历史（保留，但其结论已被推翻）**：首次运行 27 条 evidence → 30 个引用全部可定位，
> 但人工逐条判读发现 11 处指向错误（如 `trajectory/src/storage.rs:911` 实为 `source_entity_id`、
> `serde_json::to_string` 在 `:913`），随后按行号逐条修正。
> 教训仍然是有效的：**「脚本报 ✅」只证明行号存在，不证明它指向对的内容**。

## 5. 改动清单

### 5.1 第一轮（A / B / C 实施）

| 文件 | 类型 | 说明 |
|---|---|---|
| `crates/harness/src/page_type.rs` | 改 | A1：词汇表 + 亲和度公理 + 严格解析 + 3 校验器 + 11 测试 |
| `crates/harness/src/graph_dtos.rs` | 改 | A2：`PageTypeLookupError` + `unresolved_types` + warn + 委派查询 + 4 测试 |
| `crates/harness/src/knowledge_graph.rs` | 改 | B1/B2：关系词表 + 前缀族 + 实体类型 + 校验器 + `GraphNodeType` 接线文档 |
| `crates/harness/src/workflow_port_axioms.rs` | **新增** | C1：端口公理 |
| `crates/harness/src/lib.rs` | 改 | 注册 `workflow_port_axioms` |
| `crates/dao/src/repo/conversation.rs` | 改 | 2 处字面量 → 契约常量；2 处 `node_type` → 枚举 |
| `crates/dao/src/repo/knowledge_graph.rs` | 改 | `upsert_relation` 加校验 + 3 处 `node_type` → 枚举 |
| `crates/dao/src/knowledge_crud_repository.rs` | 改 | 1 处 `node_type` → 枚举 |
| `crates/trajectory/src/storage.rs` | 改 | 1 处 `node_type` → 枚举 |
| `src/commands/knowledge.rs` | 改 | 导入前关系校验（去重上报）+ 1 处 `node_type` → 枚举 |
| `src/commands/knowledge_graph.rs` | 改 | `"mentions"` → 常量 |
| `src/commands/workflow_template.rs` | 改 | C1 接线（端口公理 → warnings） |

### 5.2 第二轮（待裁决 5 项）

| 文件 | 类 | 说明（对应 §6 条目） |
|---|---|---|
| `crates/harness/src/page_type.rs` | 改 | 6.1：`PageType` +6 变体（8→14）、`ALL_PAGE_TYPES` / `SAME_TYPE_FALLBACK_TYPES` 同步、`as_str`/`parse_strict` 补 arm、模块文档新增「D3 词汇表对齐」整节、测试改造 + 新增 1 |
| `crates/agent/src/wiki_compiler.rs` | 改 | 6.1：`is_valid_page_type` 由**第二套词表**改为委派 `PageType::parse_strict`（唯一权威）；`test_is_valid_page_type` 同步（`note` 由拒→接受） |
| `crates/harness/src/graph_dtos.rs` | 改 | 6.1 + 6.5：等价性锁矩阵改用 `ALL_PAGE_TYPES`；新增 `UnresolvedTypeStat` / `collect_unresolved_nodes` / `aggregate_unresolved_stats` / `collect_unresolved_type_stats`；`GraphData` 加字段 + `new()` / `refresh_unresolved_types()`；`LinkGraph` 改为复用同一判据 |
| `crates/harness/src/knowledge_graph.rs` | 改 | 6.2 + 6.3：删 `GraphSourceType` / `CreateMultiSourceEntityInput` / 3 个默认方法；`GraphNodeType` 4→2；11 个 trajectory 变体改 `Literal`；`contains` 两条合并；`relation_table_ok` 唯一键 `(id,encoding)`→`id`；新增数据驱动列三类登记；`ENTITY_TYPE_DECLS` 7→11；模块文档新增「D5 编码收敛」整节 |
| `crates/trajectory/src/storage.rs` | 改 | 6.2：写入端改 `Display`；新增读端 `parse_stored_relation_type` / `strip_json_quotes`；新增 2 个纯函数测试（两态兼容 + UTF-8 边界安全） |
| `crates/harness/src/workflow_port_axioms.rs` | 改 | 6.4：新增 `PortAxiomSeverity` + `severity()`；`SwitchEdgeHandleNotACase` 精度修正（可达集加 `default_case`）；测试 15 → **17**（新增 2：分级锁定 / 具名默认边非死链） |
| `crates/harness/examples/port_axiom_audit.rs` | **新增** | 6.4：用生产实现扫模板 JSON 的证据工具（退出码 0/1/2，扫 0 个即失败） |
| `src/commands/workflow_template.rs` | 改 | 6.4：按 `severity()` 分流进 `errors` / `warnings`（分级判据不在命令层另写一份） |
| `src/commands/opc_workflows/seed_production.rs` | 改 | 6.4：`s-gonogo` 的 `default_case` + 2 条出边 handle 由 **value** 改 **label**（真缺陷修复） |
| `src/commands/opc_workflows/mod.rs` | 改 | 6.4：`OPC_TEMPLATE_VERSION` **3 → 4**（触发重种子） |
| `src/commands/wiki.rs` | 改 | 6.5：`get_wiki_graph_cached` 融合后 `refresh_unresolved_types()`；`GraphData` 字面量改构造函数 |
| `crates/dao/src/repo/note.rs` | 改 | 6.5：`get_vault_graph` 改用 `GraphData::new`（统计随图一并进缓存） |
| `crates/dao/src/repo/louvain.rs` / `crates/agent/src/relevance.rs` | 改 | 6.5：测试/GD 构造点改用 `GraphData::new`（新增字段后不得再散落字面量） |
| `src/components/wiki/GraphView.tsx` | 改 | 6.5：新增 `UnresolvedTypeStat` + `GraphData.unresolvedTypes?` |
| `src/pages/WikiGraphPage.tsx` | 改 | 6.5：新增「未识别类型」警示条（`Alert` banner） |
| `src/i18n/locales/*.json`（**11 个**） | 改 | 6.5：新增 `wiki.graph.unresolvedTypes.{title,desc,list}` |

**未改动（刻意）**：`crates/rt-workflow/tests/rhai_syntax_check.rs`、`stock_workflow/sim_hook.rs`、
`rhai_pm.rs` —— 属**并发会话**，本轮不碰（`cargo fmt --all --check` 在 `rhai_syntax_check.rs` 上的 diff 与本次无关）。

**备份**：
- `output/backup-2026-09-14d/`（第二轮，11 个 `.rs` 扁平命名 + `locales-before/` 11 份 JSON）。
  自证：`UnresolvedTypeStat` / `PortAxiomSeverity` / `is_data_driven_column_value` / `DataDrivenColumnDecl`
  在备份里命中 **0**、在现存里命中 **>0**；`KnowledgeDocument` 备份 0 / 现存 4 ⇒ 证明备份取自**改动落盘前**。
- `output/backup-2026-09-14/a/`、`b/`（第一轮；其中 3 份是中途快照，见 §7.7）。


### 5.3 第三、四轮（引用腐烂复查 + 接 CI）

| 文件 | 类 | 说明 |
|---|---|---|
| `crates/harness/src/knowledge_graph.rs` | 改 | 修 **4 条** evidence 腐烂引用；`DATA_DRIVEN_COLUMN` 的断言由「锁死行号」改为「按 evidence 行号读真实文件、断言该行在讲 `rtype`」（§4-B） |
| `scripts/check-decl-evidence.mjs` | **新增** | 引用腐烂校验器：`LOCATED`/`NONLOC`/`BROKEN` 三分类、扫全部引用（含括注第二条 + 裸 `:行号`）、按 id 配对输出、软判据「内容与 id 同源」、`--selftest` 8 例 |
| `scripts/ci-check.mjs` | 改 | 接 CI（本地镜像）：新增 2 条 step（`--selftest` + `--ci`），位于既有静态门禁之后 |
| `.github/workflows/ci.yml` | 改 | 接 CI（真实）：`frontend-check` 新增 `Check declaration evidence references (rot detection)`，命令与本地镜像**逐字同源** |
| `output/verify-ci-wiring.mjs` | **新增**（一次性） | 接线自校验：命令两处都在 / 未新增两侧不一致 / YAML 合法且 step 内容逐字相等 / 调度位置断言 |
| `output/verify-evidence.mjs` | **删** | 被 `scripts/check-decl-evidence.mjs` 取代（旧版恒定假红 + 静默漏扫 + 输出无 id） |

## 6. 待裁决 5 项 —— 已按推荐值实施（2026-09-14，第二轮）

> **裁定来源**：用户「**待裁决 5 项按照推荐值进行修改**」。
> 每项的「推荐值」都不是拍脑袋，而是**先实测（真 DB + 真模板 JSON + 生产实现扫描）再定**；
> 下表把「决定 / 实测依据 / 影响」逐项列清。**行为变更集中在 §6.6**。

### 6.1 D3 → 补变体（认可写入值），并把 3 套词表收敛为 1 套

| 项 | 内容 |
|---|---|
| **决定** | `PageType` **补 6 个变体**（`Doc` / `Daily` / `Knowledge` / `KnowledgeDocument` / `Synced` / `Log`），并把第二套词表 `agent/src/wiki_compiler.rs::is_valid_page_type` 改为**委派** `PageType::parse_strict` |
| **实测依据** | ① DB `notes.page_type` 分布实测：`''` 48575 行（与 NULL 混合）、`doc` 15 行 —— 词表外**真实存在**的值只有 `doc` 这 1 个；② `grep -rn "page_type: Some("` 得 7 处写入取值（`knowledge`×2、`synced`、`knowledge_document`、`doc`、`daily`、`page_type.clone()`）；③ `is_valid_page_type` 与 `page_type_dir` 各是一套独立词表 ⇒ 同一概念 3 处定义 |
| **为什么认可写入值** | 改写入端会**丢信息**（`doc` 与 `knowledge_document` 是不同来源的语义）且需数据迁移；补变体只是让词表承认既有事实。且 `notes.page_type` 会被 `dao/src/repo/note.rs:543` 当作 `node_type` 直接进图谱（NULL → `"note"`）⇒ 词表不认它，降级路径就永远在跑 |
| **影响（已核）** | `ALL_PAGE_TYPES` 8 → **14**；`SAME_TYPE_FALLBACK_TYPES` → **11**（6 个新变体全落同类型兜底 `0.3`）；跨类型仍 `0.0`。对存量图谱**零影响** —— 真实越界值只有 `doc` 一个，不存在「成对的两个不同越界值」，故「两个越界值之间」从 `0.3` 变 `0.0` 这条差异**在存量数据上取不到**（该论证已写进等价性锁的注释） |
| **附带修正** | `test_parse_strict_rejects_out_of_vocabulary` 的「越界样本」原本用了 `log` / `daily` —— 它们升级为合法值后该测试就不再测越界了，已换成真正越界的 `wiki_page` / `Entity ` / `""` 等；并**新增** `test_observed_write_values_are_in_vocabulary`（锁 7 个写入值必须在词汇表内，且不得只映射成 `Unknown`，带反向对照） |

### 6.2 D5 → 写入端收敛 + 读取端永久兼容（**零迁移**）

| 项 | 内容 |
|---|---|
| **决定** | 写入端收敛为**裸字面量**；读取端**永久兼容两态**；**不写迁移脚本** |
| **实测依据** | `select count(*) from knowledge_relations where relation_type like '"%'` = **0**；`count(distinct relation_type)` = **56**；`count(*)` = 112937 ⇒ 存量**没有**带引号行 |
| **为什么仍要读端宽容** | 本机 = 0 只说明**这台机器**没写过；已发布版本的 `serde_json::to_string` 在别的机器上写过。读端宽容 ⇒ 存量行**自愈**；迁移脚本漏一次就永久丢数据。判据：**写入端收敛 + 读取端宽容 = 无迁移收敛** |
| **改动** | ① `trajectory/src/storage.rs::save_relationship`：`serde_json::to_string(&rel.relation_type)` → `rel.relation_type.to_string()`（`Display`）；② 新增读取端 `parse_stored_relation_type` + `strip_json_quotes`（两态兼容 + UTF-8 边界安全）；③ `RELATION_DECLS` 的 11 个 trajectory 变体全部 `JsonEncodedVariant` → `Literal`；④ `contains` 的两条登记**合并为一条**（两条链路的出处都留在 `evidence` 里）⇒ 总数 **19 → 18** |
| **唯一键收紧** | `relation_table_ok` 的唯一键由 `(id, encoding)` 改为 **`id`** —— 原宽松度在收敛后变成漏洞：有人再引入第二套编码时表能过、而 `relation_decl` 只返回第一条 ⇒ 另一条链路静默消失。已加负向对照「同一 id 两条登记（编码不同）必须红」 |

**⚠ D5 实施中发现的第二类存量数据（本条是本轮最意外的发现）**

实测 `relation_type` 列里除了代码常量，还有 **22 个裸中文职位名 / 约 6 万行**：

| 值 | 行数 |
|---|---|
| `董事` | 21700 |
| `独立董事` | 17876 |
| `employ_董事` | 10850 |
| `董事长` | 5636 |
| …（共 22 个 distinct） | 合计约 6 万 |

来源是 `knowledge-sources/lemonhu/edges.csv` 的 `rtype` 列（`src/commands/knowledge.rs:1725` 原样入库）。
`RELATION_DECLS` 是照「代码常量」建的，**完全没盖住这 6 万行** ⇒ 校验一上线就对存量数据刷屏，等于没做。

⇒ 新增**第三类登记**：`DataDrivenColumnDecl` / `DATA_DRIVEN_COLUMN`（记录列、数据文件、出处）
+ 形态判据 `is_data_driven_column_value`（**含非 ASCII 字符即放行**），接入 `is_declared_relation` 的第三支。

> **为什么按形态而不是逐条登记**：数据文件换一版值域就变，逐条登记必然腐烂；
> 而 ASCII 形态的值只可能来自代码常量（可穷举）⇒ **不放行**，仍要求逐条登记。
> 这样「数据驱动的开放词表」与「代码里的封闭词表」在判据上被分开，而不是把整个校验放水。
> 正反例都锁在 `test_data_driven_column_values_are_shape_gated`：`董事` 放行 /
> `totally_made_up` 必须报出（**且先用 `relation_family_of(v).is_none()` 排除前缀族路径**，
> 否则「放行」可能来自 `employ_` 族而不是这条判据 —— 测不到想测的东西）。

**`ENTITY_TYPE_DECLS` 同步由真实数据修正**：`person` / `organization` 原为「文档示例」（`observed: false`），
实测该列有 `person` 62616 / `company` 9564 / `industry` 146 / `concept` 487 —— 均来自
`knowledge-sources/lemonhu/nodes.csv` 的 `type` 列（20872 / 3188 / 49 行）。故：

- `person` / `company` / `industry` 改 `observed: true`（带数据文件与 DB 双出处）；
- 新增 `COMPANY`（DB 2 行）/ `CONCEPT`（DB 1 行）—— **大小写不一致**形态，登记为**可见**而不静默归一
  （静默归一 = 改动存量数据）；
- `organization` 成为**唯一悬空**项（零写入点、DB 实测 0 行）；
- 总数 **7 → 11**。

### 6.3 B2 → 裁剪（删声明），而不是实现多源接入

| 项 | 内容 |
|---|---|
| **决定** | **删除** `GraphSourceType`（4 值）、`CreateMultiSourceEntityInput`、`EntityGraphProvider` 的 3 个默认方法；`GraphNodeType` 由 4 变体裁到 **2**（`Entity` / `Note`） |
| **实测依据** | ① `GraphSourceType`：仅 3 处定义内自引用 + 1 处字段声明，**定义外 0 引用**；② `CreateMultiSourceEntityInput` 与 3 个默认方法（`get_entities_by_source` / `create_multi_source_entity` / `search_entities_by_node_type`）：**0 调用、0 覆写**；③ `EntityGraphProvider` 只有 2 个实现者（`dao::knowledge_graph_provider`、测试用 `NoopEntityGraphProvider`）；④ DB 实测 `knowledge_entities.node_type` **只有 `entity` 一种**（72816 行）⇒ 多源接入**没有真实数据支撑** |
| **为什么删而不是做** | 实现多源接入需要新的写入方 + DB 列 + 迁移，是产品方向选择；而当前形态是「声明齐全、零消费」——正是上一轮被批评的形态。**裁剪掉声明的成本 = 0** |
| **恢复判据（写进源码）** | 同一接口必须**同时**具备 **≥1 调用方 + ≥1 覆写实现**才算活接口 —— 只有其中一个都是「看起来在设计，其实没人用」 |

### 6.4 C1-升级 → 分「error / warning」两级，并按引擎语义修出一个真缺陷

| 项 | 内容 |
|---|---|
| **决定** | 违规分两级：`Error` = 该边/该分支**永不激活**（结构性死链）；`Warning` = 边会激活但**有两个答案**（语义冲突）。`Error` 进 `ValidationResult.errors`（挡保存），`Warning` 进 `warnings`（不挡） |
| **分级判据依据** | 引擎行为（`dag_store.rs:100-121`）：落不到任何分支 = 该边**永不激活**；落两条分支 = 边会激活但答案不唯一 |
| **实证扫描** | 用**生产实现** `validate_port_axioms` 扫 DB 全量模板导出（**111 个模板 / 956 个节点 / 0 个解析失败**）：**3 处 error**（1 × `switch_edge_handle_not_a_case` + 2 × `switch_case_unreachable`），**全部落在唯一一个模板** `prod-startup-mvp`；对照 `stock-analysis::quality-gate`（`expression` 模式、label-only handle、`default_case=low-quality`、含无 handle 边）**零违规** ⇒ **分级不误伤存量** |
| **⚠ 扫描数从 4 变 3（精度修正排除了 1 个误报）** | 第一轮（精度修正**前**）报 **4 处**：2 × `switch_edge_handle_not_a_case` + 2 × `switch_case_unreachable`。第一轮同时把 `SwitchEdgeHandleNotACase` 的可达集修正为 `{case.label} ∪ {default_case}` ⇒ 重新扫描后 `e-nogo`（handle `"no-go"` == `default_case` `"no-go"`）**不再报** —— 它是**合法的具名默认绑定**，不是死边。**这是我自己工具箱里的误报，被我自己第二轮修掉了** |
| **真缺陷（已读引擎五处实现 + DB 行确认）** | DB 实测 `prod-startup-mvp` v3 的 `s-gonogo`：`cases = [{label:"GO 上线",value:"go"},{label:"NO-GO 打回",value:"no-go"}]`，`default_case = "no-go"`（**value**），两条出边 handle = `"go"` / `"no-go"`（**value**）。引擎（`switch_executor.rs:180-377` 的 `use_llm` / `expression` / 传统三个分支）写进 `matched_label` 的**都是 `case.label`** ⇒ **命中任一 case 时两条出边都匹配不上 ⇒ 两条分支的「正确路径」都断**。而 `actual=None`（无 case 命中）时 `matched_label = default_case = "no-go"`，**恰好等于** `e-nogo` 的 handle ⇒ NO-GO 边被激活 |
| **净效果（比「分支全灭」更准）** | **GO 分支永远不可达**；NO-GO 分支只在「**无 case 命中**」时被激活 —— 而它本该在「命中 `NO-GO 打回` 这个 case」时激活。两条错误的值（handle 与 `default_case` 都用 value）**互相抵消出一条畸形通路** ⇒ 流程看起来「没完全断」⇒ **缺陷被掩盖**。这正是「两个缺陷互相掩盖」的形态：只修其中一处（例如只改 handle）反而会让 NO-GO 通路也一起断掉 |
| **修复** | `seed_production.rs`：`default_case` → `"NO-GO 打回"`、两条出边 handle → `"GO 上线"` / `"NO-GO 打回"`（一律用 **label**，`value` 保持机器比对值不动）⇒ 两条边各按自己的 case 命中，且无 case 命中时按 `default_case` 走 NO-GO。`OPC_TEMPLATE_VERSION` **3 → 4** |
| **为什么必须升版本号** | `check_template_version` 见 `existing.version >= version` 即**跳过种子**（`opc_workflows/mod.rs:629`）⇒ 不升版本，修复在已装库上**静默失效**。DB 实测该模板当前 `version = 3`，故应用下次启动重种子会覆盖 |
| **同时修正公理精度（重要）** | Switch 出边 handle 的**可达集 = `{case.label} ∪ {default_case}`**，不是只有 `label`。`default_case` 是引擎赋给 `matched_label` 的值之一 ⇒ 漏掉它会把「合法的具名默认边」误判成死链。**升级成 `error` 后，误报的代价从「误导」升级为「挡人」** ⇒ 必须同时收紧精度。已加测试 `test_switch_named_default_handle_is_not_a_dead_edge`（handle == `default_case` 不报 + 负向对照 `go-live` 必报） |
| **新增证据工具** | `crates/harness/examples/port_axiom_audit.rs` —— 用生产实现扫模板 JSON，退出码 0/1/2，且**输入 0 个模板即判失败**（防「扫 0 个文件报通过」，见判据 #7 同类） |
| **证据可复现** | ① 导出模板：`select json_agg(row_to_json(t))::text from (select id, nodes::text, edges::text from workflow_templates) t;` 写入 `output/tpl-dump.json`（`nodes`/`edges` 是 **text 列**，故审计工具内部需二次 `from_str`）② `cargo run -p axagent-harness --example port_axiom_audit -- output/tpl-dump.json` ③ 反事实：`node output/patch-dump-for-counterfactual.mjs` 生成 `tpl-dump-patched.json` 再跑一次。（两个 dump 共 3.2 MB，验证完成后已删除，用此法可再生） |

### 6.5 A2-升级 → 进 UI（未识别类型规模可见）

| 项 | 内容 |
|---|---|
| **决定** | `GraphData` 新增 `unresolved_types` 字段并送到界面 |
| **DTO 形态** | `#[serde(default, skip_serializing_if = "Vec::is_empty")] unresolved_types: Vec<UnresolvedTypeStat>`，`UnresolvedTypeStat { rawType, count, sampleNodeIds }`（样例最多 3 个，按 id 升序） |
| **兼容性（零迁移）** | `default` ⇒ **旧版缓存 JSON 仍能解析**（字段缺失即空清单）；`skip_serializing_if` ⇒ 正常图不写该字段。与既有 `GraphNode::link_count` 的约定一致。**`GraphData` 是手写 `Serialize/Deserialize`（非 ts-rs）** ⇒ 后端加字段即可，旧缓存也不会解析失败 |
| **判据只有一份** | 新增 `collect_unresolved_nodes` / `aggregate_unresolved_stats` / `collect_unresolved_type_stats`；`LinkGraph::from_graph_data` **改为调用同一函数**（原先自己写了一版 `filter(parse_strict().is_none())`，与统计函数是**两份判据**） |
| **接线点（单点）** | `src/commands/wiki.rs::get_wiki_graph_cached` 的**融合之后**（`base_graph.refresh_unresolved_types()`）。选这里的两条理由：① 统计必须对**最终返回给前端的节点集**成立（`get_vault_graph` 算的那次还没融合实体节点）；② **缓存命中分支读的是 DB 里存的 JSON**，那份可能是升级前写的（缺字段）⇒ 不在此重算，老缓存就永远看不到提示 |
| **前端** | `GraphView.tsx`：新增 `UnresolvedTypeStat` + `unresolvedTypes?`（`?` 是刻意的 —— 后端空清单时**不写该字段**）；`WikiGraphPage.tsx`：新增警示条（`Alert type="warning"` banner，`stats` 计算之后） |
| **i18n** | 新增 3 个 key（`wiki.graph.unresolvedTypes.title/desc/list`）× **11 个 locale 全部补齐**；校验：11 份 JSON 语法 + 插值变量（`{{nodes}}`/`{{types}}`/`{{list}}`）逐一核对 **PARITY-OK** |
| **副作用（正向）** | §7 遗留 1（「A 组两个查询 API 暂无外部调用方」）中的 `unresolved_types` 部分**已消除** —— 现在它有真实生产消费方（UI） |

### 6.6 ⚠ 行为变更提醒（必须显式知晓）

| # | 变更 | 具体后果 | 风险控制 |
|---|---|---|---|
| 1 | **端口公理 4 类升为 `error`** | `is_valid = errors.is_empty()` ⇒ 含这 4 类违规的模板**保存会被挡下**（UI 上表现为校验失败） | 实测 111 个存量模板中**只有 `prod-startup-mvp` 命中**，且它命中的是**真缺陷**（已修种子）。其余 110 个零违规 |
| 2 | **`OPC_TEMPLATE_VERSION` 3 → 4** | 应用升级后下次启动会**重新种子**这几个 OPC 模板 ⇒ **覆盖用户对它们的任何手工修改** | 影响面 = `seed_production.rs` 的 2 个模板（该常量无其它使用者）。这是**必须付**的代价：不升版本号，缺陷修复在已装库上静默失效 |
| 3 | **`is_valid_page_type("note")` 由 `false` → `true`** | wiki 编译器的页面类型白名单**放宽**：此前被拒的 `note`（与 6 个 D3 新值）现在被接受 | 依据：`note` 是 `notes.page_type` 为 NULL 时的**默认值**（`dao/src/repo/note.rs:543`），拒绝它等于与 DB 事实矛盾。测试已同步（原断言 `!is_valid_page_type("note")` 反转为接受，并注释说明） |
| 4 | **`prod-startup-mvp` 的 Switch 分支绑定修复** | 该模板**实际执行路径会变**。修复前：`GO 上线` 分支永远不可达；`NO-GO 打回` 分支只在「输入未命中任何 case」时被激活（因为 `default_case` 恰好等于 `e-nogo` 的 handle）⇒ 记为 NO-GO 的决策**可能来自兜底而非 case 命中**。修复后：两条边各按自己的 case 命中，无 case 命中时走 NO-GO | 这正是修复目的。观察差异的方法：对比修复前后同一工作流的日志 —— 修复前**不应出现** `a-launch`（GO 后继节点）被执行；且决策为 NO-GO 时应看到它来自兜底路径（`input` 不匹配任何 case） |
| 5 | **`relation_type` 写入编码变化** | 新写入的关系在 DB 里是**裸字面量**（此前 trajectory 那条路径写的是带引号的 `"part_of"`）。**按字面量过滤的读方从此才能匹配到这些行**（旧行为是漏掉） | 读取端永久兼容两态 ⇒ 存量行不受影响；本机实测带引号行 = 0 |
| 6 | **`GraphData` JSON 新增可选字段** | 前端收到的图谱 JSON 在**有未识别类型时**多一个 `unresolvedTypes`；无则不出现 | `#[serde(default)]` ⇒ 双向兼容（旧前端忽略新字段、新前端读旧缓存不炸） |


## 7. 遗留（如实登记）

1. ~~**A 组两个查询 API 暂无外部调用方**（`get_page_type_strict` / `unresolved_types`）~~
   —— **`unresolved_types` 已于第二轮消除**：现在有真实生产消费方（`get_wiki_graph_cached` 回填 →
   `WikiGraphPage` 警示条）。`get_page_type_strict` 仍只有测试在调用（如实保留）。
2. **`relation_decl` 的 API 形状已基本失去踩坑面**：唯一键改成 `id` 后，同一 id 恒有 0 或 1 条登记
   （`relation_table_ok` 机器强制 + `test_relation_ids_are_unique` 复核）。`relation_decls`（取全部）**刻意保留**
   作回归保险 —— 若哪天又出现同 id 两条登记，这个入口能让它立刻可见。
3. **C 组覆盖边界（第二轮已大幅扩展）**：用**生产实现**扫了 DB 全量 **111 个模板 / 956 个节点**（0 解析失败），
   `error` 级违规只剩 `prod-startup-mvp`（已修种子 + 升版本号）。仍**未**逐一人工判读「未声明 `default_case`
   的 Switch」是否语义正确 —— 公理只判「结构性死链 / 双激活」，判不了「这个 case 划分是否合理」。
4. **`EdgeType` 同名两份**：`workflow_types::EdgeType`（9 值的 DAG 边类型）与
   `workflow_graph::EdgeType`（路由树层级关系）**同名不同义**，同 crate 内共存 —— 新发现的「一名多义」点，登记待清理。
5. **`warningType` 在前端是死字段**（第二轮后**更值得修**）：`workflow.types.ts:792` 与 `DebugPanel.tsx:318`
   各声明一次，**全前端读取 0 处**（`DebugPanel.tsx:1134` 只渲染 `warn.message` + `nodeId` Tag）。
   后果：C1 的 6 类违规**在 UI 上不可区分**，只能读 message 文本；好处是**不产生 i18n 义务**（已核：无按 warningType 的 key 映射）。
   第二轮后 `error` 级会真正**挡住保存**，用户更需要知道「是哪一类」⇒ 建议在 `DebugPanel` 里真正消费 `warningType`
   （独立小改动，本轮未做）。
6. **`PortAxiomViolation::describe()` 的语言**：`commands/workflow_template.rs` 里既有全部 20 条
   `ValidationWarning.message`（`:645`/`:655`/`:750`/`:834`…）**都是英文**，且该字段由 `DebugPanel.tsx:1134`
   **原样渲染** ⇒ 混用会让同一个警告列表两种语言并存。已保持英文并把语言约定写进 doc。
   （B 组的违规消息走 `tracing::warn!`，是内部日志、不向前端暴露，中文与项目日志一致 —— 无需改动。）
7. **备份集有 3 份是「中途快照」而非改造前快照**（第一轮 `b/` 目录：`dao_repo_conversation.rs` 只有
   `node_type` 未改、`commands_knowledge_graph.rs` 的 `mentions` 已改、`b/harness_knowledge_graph.rs` 缺最后的测试修正）。
   `a/` 两份与 B 组另 4 份经「grep 新符号命中数 = 0」验证为**真改造前**。
   影响可控，但**不可直接 `cp` 恢复**。教训：**备份必须自证是改造前**。
   第二轮 `output/backup-2026-09-14d/` 已按此判据自证通过。

### 7.1 第二轮新增的待裁决（**未擅自改**）

| 编号 | 事项 | 先决问题 |
|---|---|---|
| **D3-b** | 前端 `GraphNodeType` 联合类型（**4 值**）是「**有配色的类型集**」，不是后端 `PageType`（**14 值**）的镜像 —— 其余 10 个值在 `graphViewUtils.ts:191` 落 `typeMap[node.type] \|\| typeMap.note` ⇒ **静默显示成 `note` 色**，与真正的笔记不可区分 | 是否为新类型配色 / 加 `wiki.graph.nodeType.*` 标签（= 要选色 + 要翻译 11 语言）。**已修的部分**：标签渲染的缺失 key 兜底（`t(key, { defaultValue: meta.type })`）—— 那是纯 bug（会把 key 原文 `wiki.graph.nodeType.doc` 当文案显示），不涉产品决策 |
| **D5-b** | `knowledge_entities.entity_type` 存在**大小写不一致**形态：`COMPANY`（DB 2 行）/ `CONCEPT`（DB 1 行）与 `company` / `concept` 并存 | 静默归一（改存量数据）vs 只登记（已做）vs 修写入端。**已做的**是登记为可见（`observed: true` + `meaning` 标注），不归一 |
| **B1-b** | `relation_type` 列里 22 个**裸中文职位名 / 约 6 万行**（数据驱动列）按「含非 ASCII 即放行」处理 | 更严格的方案是**按数据文件白名单**（读 `edges.csv` 的 distinct 值）—— 但那让判据依赖运行时文件，且文件更新后仍需重新同步 ⇒ 当前选了形态判据。若要更严，需产品确认「未在数据文件里出现的中文值是否应报警」 |
| **C1-b** | `PortAxiomSeverity::Error` 目前含 4 类。`HandleContradictsEdgeType`（handle 与 edge_type 推出的兜底分支不一致）留在 `Warning` | 它确实是「同一条边两个答案」⇒ 边会激活，故不是死链。但若认为「自相矛盾的绑定」也必须挡保存，就要升为 `Error`。实测：存量 111 模板中该类的数量见审计输出 |

