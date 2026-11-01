# 执行记录：本体适应性接线 P0–P3（2026-09-14）

> 上游：`AUDIT-ontology-recap-adaptive-wiring-2026-09-14.md` §4「建议接线清单（按性价比排序）」
> 指令：**「按照 p0、p1、p2、p3 顺序执行」**（= 授权按推荐值实施全部接线项）
>
> 纪律：git 零接触；进 UI 的文案一律英文（i18n 走 5 语言 key）；**行为变更显式提醒**；
> 引用一律核到当前真值；**报告不擅改**（历史报告只登记不修改，见 §8）。
> 本文所有数字均为**本轮实测**，未沿用既往记录。

---

## 0. 一页结论

| 项 | 交付 | 状态 | 关键实测 |
|---|---|---|---|
| **P0** | 端口公理 `Error` 下沉到**所有**写入路径（用户/LLM/导入 = 硬拦；种子 = 软门禁） | ✅ 前序已交付，本轮复跑 | 15/15 写入点已接，`未接 0` |
| **P1** | 实体类型「写端观测 + 读端归一」成对 | ✅ 读端前序 / **写端本轮** | 4 个写入点接入；纯函数测试 3 例 |
| **P2-a** | OPC `entity_types` 与 KG `ENTITY_TYPE_DECLS` 值域不相交 | ✅ 前序已交付 | 边界测试锁死 |
| **P2-b** | 边侧可观测化 | ✅ **本轮**，且**发现真缺陷** | **112937 行 / 56 个关系类型 → 1 个常量**（信息量归零） |
| **P2-c** | 前端图例跟随词表 | ✅ **本轮** | 图例 top-8 + 边侧 Alert；5 key × 11 locale |
| **P3** | `search` 层是否应消费本体 | ✅ **本轮取证，结论=不接线** | 能力已实现但**零非空生产者** |
| **附加** | 引用卫生（本轮编辑造成的引用腐烂） | ✅ **本轮** | 19 条引用修复；检查器补覆盖率自证 |

**最重要的一个发现（P2-b）**：边侧的问题**不是**「前端配色没跟上」，而是
`get_knowledge_graph_edges_for_wiki` 把 `relation_type` **读出来又扔掉**
（`edge_type` 恒为常量 `"reference"`）⇒ DB 里 **112937 行、56 个不同关系类型
全部塌成同一个值**。这是功能级的信息丢失，不是显示问题。

---

## 1. P0 —— 端口公理硬拦（前序已交付，本轮复核）

### 做了什么

| 层 | 入口 | 语义 |
|---|---|---|
| 判据（**只此一份**） | `PortAxiomSeverity::severity()` | 4 类 `Error`（结构性死链）/ 2 类 `Warning`（语义冲突） |
| DAO 硬拦 | `workflow_template.rs` 的写入函数 → `port_axiom_errors` / `enforce_port_axioms` | `Err` 阻断保存 |
| 种子软门禁 | 9 处种子 + `warn_port_axioms` / `warn_port_axioms_json` | 日志可见但**不让应用起不来**（种子非法是开发期 bug） |
| 存量自愈 | `reset_port_axiom_illegal_versions` | 把非法行 `version` 归零，使散落 14 处的 `existing.version >= TEMPLATE_VERSION` 版本门自然放行重建 |
| 接线守卫 | `scripts/check-port-axiom-wiring.mjs` | 枚举所有写 `workflow_templates.nodes/edges` 的函数，未接即红 |

`warn_port_axioms*` / `enforce_port_axioms` 全是 `validate_port_axioms` + `severity()` 的薄封装
—— **同一判据不得写两份实现**。

### 本轮复核（`node scripts/check-port-axiom-wiring.mjs`）

```
汇总：已接 15｜未接 0（写入点函数合计 15）
PASS：所有写 workflow_templates.nodes/edges 的函数都已接门禁。
WIRING_EXIT=0
```

### ⚠ 行为变更提醒（P0 唯一会让用户感知的一条）

`prod-startup-mvp` 等**存量非法模板**：下次保存会**失败**（硬拦）。
自愈逻辑已把非法行的 `version` 归零 ⇒ 启动时会按当前版本**重建**，重建后即合法。

---

## 2. P1 —— 实体类型：写端观测 + 读端归一

### 为什么必须成对做

「登记」只让 `validate_entity_type` **不再报**它；而读端原本用精确
`== "company"` / `== "concept"` 比较字面量 ⇒ DB 里的大小写变体
（实测 `COMPANY` 2 行 / `CONCEPT` 1 行）**恒 false** ⇒ 这些实体不会被注册成概念节点、
成员关系也建不起来，**全程静默无报错**。只登记不归一 = 「问题可见但没人修」。

### 读端（前序会话）

`analysis-engine/src/knowledge_loader.rs`：新增 `normalized_entity_type()`（未登记 ⇒ `None`）
与 `is_concept_like()`，三处精确比较（`:73/87/90`）全部改为经归一比较。

### 写端（**本轮**）

判据单份 + 四个写入点全覆盖：

| 写入点 | 接入方式 |
|---|---|
| `dao/repo/knowledge_graph.rs::create_knowledge_entity` | `validate_entity_type(&input.entity_type)` |
| `dao/repo/knowledge_graph.rs::upsert_entity` | `validate_entity_type(entity_type)` |
| `dao/repo/knowledge_graph.rs::batch_upsert_entities_and_relations` | 循环**前**整体 `observe_entity_types(...)`（批量去重） |
| `dao/repo/knowledge_graph.rs::upsert_relation` | 内联 6 行收敛到 `observe_violation()` |
| `src/commands/knowledge.rs`（CSV 导入**直写**，绕过 DAO） | 循环前显式调用（唯一不经过 DAO 的路径） |

新增设施（均在 `dao/repo/knowledge_graph.rs`）：

```rust
const MAX_DISTINCT_UNREGISTERED: usize = 10;
fn observe_violation(context: &str, v: Option<GraphTypeViolation>);
pub fn unregistered_entity_types<'a>(types) -> (Vec<String>, usize);  // 纯函数、可测、去重 + 自报截断数
pub fn observe_entity_types<'a>(context: &str, types);                // 日志只做出口
```

**设计取舍**：日志**只做出口**（不阻断写入）—— 存量 DB 里确有未登记值，
硬拦会让导入直接失败。「让问题可见」是本项的**全部**目标，修不修是产品决策。

测试 `entity_type_observation_tests`（3 例）：正负对照 / 去重 / 上限+截断自报
（含算术闭合与「恰好在上限不虚报」反向对照）。

---

## 3. P2-b —— 边侧：从「前端没配色」查到「出口把数据扔了」

### 3.1 起点与真实缺陷

原假设：前端 `GraphEdgeType` 与后端产出的边类型对不上 ⇒ 前端配色漏了。

**实测否定了这个假设，并查出更严重的问题**：`GraphEdge` 只有**一个** `type` 字段，
而 `get_knowledge_graph_edges_for_wiki` 里把这一个字段写死成常量：

```rust
// 改造前
edge_type: "reference".to_string(),   // 从 relation_type 读出来的关系信息被丢弃
```

⇒ 渲染类别与本体身份被迫共用一个字段，结果**本体身份全丢**。

### 3.2 实证（`examples/wiki_edge_relation_probe.rs` 四步探针，`EXIT=0`）

```
① kb=f9b2b050-…｜边 38171 条
   改造后：distinct relationType = 24｜type 分布 = {"reference": 38171}
   反事实（旧实现）：distinct relationType = 0｜distinct type = 1  ← 信息量归零
   top5：董事×10850｜has_concept×9442｜独立董事×8938｜in_industry×3166｜董事长×2818
① kb=lemonhu_knowledge_graph｜边 74766 条
   改造后：distinct relationType = 55｜type 分布 = {"reference": 74766}
   top5：has_concept×18884｜employ_董事×10850｜董事×10850｜employ_独立董事×8938｜独立董事×8938
合计：边 112937 条｜改造后 distinct 标签 56 个｜反事实 distinct 标签 0 个｜blank 0 条
③ GraphData::new：unresolved_relations = 0 项
④ 注入未知标签后：unresolved_relations = 1 项（count=2）
✅ 四步全部符合预期
```

> 探针**含反事实复现**：同一步里同时跑「现状」与「旧实现」，否则「改造有效」只是断言。
> 第一版探针把断言方向写反（要求旧实现**不含** `<none>`），每库都误报失败 —— 已修，
> 并在代码里注明该坑。

### 3.3 改造：把两个语义拆成两个字段

| | `GraphEdge.type` | `GraphEdge.relation_type` |
|---|---|---|
| 语义 | **渲染类别**（决定「画成什么样」） | **本体关系 id**（决定「这条边是什么关系」） |
| 值域 | `link` / `reference` / `mapping` | `has_concept` / `董事` / `employ_董事` …（形态放行） |
| 判据入口 | `is_declared_edge_type` | `is_declared_relation` |
| 声明表 | `EDGE_TYPE_DECLS`（6 条） | `RELATION_DECLS` |

- `graph_dtos.rs`：`relation_type: Option<String>` + 两个构造器 `GraphEdge::structural()` /
  `GraphEdge::relation()`（后者内置 `"reference"` 字面量 ⇒ 全仓 `"reference"` 只有一处）。
- **零视觉变更**：`type` 的取值分布与改造前**逐字相同**（探针 ① 已证：全是 `"reference"`）。
- **零数据迁移**：`#[serde(default, skip_serializing_if = "Option::is_none")]`
  ⇒ 旧缓存 JSON 可解析、正常图不背字段。

### 3.4 A2（节点侧）与 P2-b（边侧）对称

| | 节点侧（A2） | 边侧（P2-b） |
|---|---|---|
| 字段 | `GraphData.unresolved_types` | `GraphData.unresolved_relations` |
| 判据 | `PageType::parse_strict().is_none()` | `uninterpreted_edge_label()` |
| 聚合 | 共用 `group_by_raw_label()`（**一份实现**） | 同上 |

- `refresh_unresolved_types()` **改名** `refresh_unresolved()`：两侧总是一起被调用，
  拆两个迟早只调一个。
- 边侧统计**刻意不复用** `UnresolvedTypeStat`（`sampleNodeIds` 在边侧会撒谎），
  单列 `UnresolvedRelationStat { raw_type, count, sample_edge_ids }`。

### 3.5 ⚠ 刻意**不**统计的一件事：「前端没有专属配色」

边侧统计只回答**后端解释不了**；「前端没有专属配色」由图例回答。
混用会让字段对存量数据**刷屏**：53 个中文关系类型 / 74325 行，而
`is_declared_relation` 对非 ASCII 按形态放行 ⇒ 它们是**合法的**，不是问题。
该说明已写进 `uninterpreted_edge_label()` 的文档。

---

## 4. P2-c —— 前端：给 `relation_type` 一个**唯一展示出口**

`src/components/wiki/GraphView.tsx` + `src/pages/WikiGraphPage.tsx`：

- `GraphEdge.relationType?: string`（**刻意不声明联合类型** —— 词表是开放的，
  写死联合类型就把「词表可扩展」变成了「改前端才能加关系」）。
- 图例 Popover 在边类型筛选之后新增「知识库关系类型」分节：`RELATION_LEGEND_TOP_N = 8`
  的分布统计 + rest 折叠。**刻意不做白名单**（做白名单就等于把审查偷偷加进来）。
- `WikiGraphPage` 新增边侧 Alert（与节点侧对称），展示 `unresolvedRelations`。
- i18n：**5 个 key × 11 locale = 55 处全部落地**（本轮实测逐 key 核验）。

```js
// 实测：11 个 locale × 5 key，全部 ✓
wiki.graph.relationTypes / relationTypesMore
wiki.graph.unresolvedRelations.title / .desc / .list
```

---

## 5. P3 —— `search` 层是否应消费本体：**结论=不接线（登记为无入边）**

### 取证（全部为当前真值）

| # | 事实 | 位置 |
|---|---|---|
| 1 | RAG 管线的图检索**硬编码空 filters** | `search/src/rag_pipeline.rs:154`=`entity_type_filters: vec![]`、`:155`=`relation_type_filters: vec![]` |
| 2 | 前端**刻意不传** filters（传参与依赖项里都没有） | `KnowledgeGraphSearchPanel.tsx:104-109` 调用体、`:110` deps 数组 |
| 3 | 过滤能力**已完整实现但零非空生产者** | `dao/src/knowledge_graph_provider.rs:95/100`（实体类型过滤）、`:104/110`（关系类型过滤） |
| 4 | `search` 层**不参与排序**：唯一本体触点是**裸值直渲** | `search/src/rag.rs:676` `format!("- {} ({})", entity.name, entity.entity_type)` |

字段定义在 `harness/src/knowledge_graph.rs:193/198`。

### 判定

- 这是判据 B「**断链（入边缺失）**」的标准形态：**能力已存在但无入边**。
- **本轮不擅自接线**：接 filters 会改变 RAG 召回面（= 改变给 LLM 的上下文），
  属产品语义决策，须先确认「按类型过滤」是期望行为。
- 附带登记（**未修**）：`rag.rs:676` 把 **`entity_type` 原样**渲进 prompt ——
  大小写变体（`COMPANY`）与未登记值会**逐字进入 LLM 上下文**。
  与 P1 读端归一是同一个根因的另一处消费端，但改它同样是产品决策（改 prompt 内容）。

---

## 6. 附加工作项：引用卫生（本轮自作自受，必须自证）

### 6.1 问题：我自己的编辑当场造成引用腐烂

本轮在 `commands/knowledge.rs`（+13 行）与 `dao/repo/knowledge_graph.rs`（+~99 行）
**插入了行** ⇒ `harness/knowledge_graph.rs` 里 6 条既有 `文件:行` evidence 立刻失效。

**而 `check-decl-evidence.mjs --ci` 报「腐烂 0」** —— 因为它的 LOCATED 判据只到
「文件在 / 行号不越界 / 不是空行」，**指到别的行同样算 LOCATED**。
语义腐烂要人 grep 才看得见。

### 6.2 修法

`output/fix-citation-rot-2026-09-14.mjs`（**幂等 + 自证**）：

- 每条要求「旧串恰好出现 1 次」；例外条目（旧串多处）允许全量替换，
  但要求**多处的目标值必须相同**（否则语义分叉会被一次 `replaceAll` 悄悄带过）。
- **幂等**：旧串已消失且新串在 ⇒ 记「已应用」，不报失败（否则第二次跑必假红）。
- **自证**：替换后新串次数必须 = **原有次数 + 替换次数**
  —— ⚠ 第一版断言写的是「== 替换次数」，而 `knowledge.rs:1782` 在 489 行**早已存在**，
  导致**正确的替换被判成失败**（审计脚本自身撒谎的又一例）。

### 6.3 两轮普查（第一轮**不够**，这是关键教训）

| 轮次 | 手段 | 结果 |
|---|---|---|
| 第一轮 | 只匹配 `evidence: "…"` 字段形态 | 修 12 条，**散文/文档注释里的同一条引用全部漏扫** |
| 第二轮 | 全仓正则普查（`(文件):(\d+)` 覆盖 code+docs） | 又查出 **7 条** |

第二轮新增（含**行号未漂但语义已反**一类）：

| 引用点 | 写的 | 真值 | 归因 |
|---|---|---|---|
| `harness:539` → `knowledge.rs:1725` | 1725 | **1738** | 本轮 +13 |
| `harness:1088/1295` → `knowledge.rs:1765` | 1765 | **1782** | 本轮 +13 |
| `harness:871` → `knowledge_loader.rs:73/87/90` | 「用精确 `==`」 | 行号**对**，但那三行**已是归一实现** | 本轮 P1 |
| `page_type.rs` ×6 → `wiki.rs:1637/1883/1383` | 1637/1883/1383 | **1650/1896/1396** | 前几轮 +12、本轮 +1 |
| `dao/examples/wiki_edge_relation_probe.rs:126`（我自己新建的） | `knowledge_graph.rs:1226` | **1349** | 本轮 |

> `page_type.rs` 那三条偏移 **13 行里有 12 行不是本轮造成的** ⇒ 说明这些引用
> **此前从未被人核过**（`wiki.rs` 里根本没有 `PageType::Daily` 构造，
> 引用指的是裸字符串 `page_type: Some("daily")` 的写入行）。

本轮合计修复 **19 条**引用（脚本 19 条，其中 11 条在应用后转为「已应用」跳过）。

### 6.4 检查器本体：补覆盖率自证（判据 #130）

**缺陷**：`scripts/check-decl-evidence.mjs:36` 的 `DECL_TYPES` 是硬编码清单，
本轮新增的 `EdgeTypeDecl`（6 条声明）**不在检查范围内** ⇒ 新表一旦写错引用，
门禁**静默放过**，脚本照样报「腐烂 0」。这**不是**「检查通过」，是「从没检查过」。

**修法**：

1. `DECL_TYPES` 改为从 `DECL_TABLES`（`{ type, konst }` **成对**）派生
   —— 分列两张清单（类型名一张、常量名一张）必然漂移，那正是原缺陷的成因。
2. 新增 `coverageGaps()` **双向**自证：源码里的每个 `pub const *_DECLS` 都要有配对类型；
   登记的类型也必须真的解析到块。任一不满足 ⇒ `exit 2`（脚本自身失效）。
3. 给守卫加 **3 例负样本对照**（含「源码多一张未登记的表」「登记了但解析不到块」）
   —— 一条从没红过的守卫本身不可信。

**修后实测**：

```
✔ 35 条引用行号可定位，且内容与所属 id 同源（白名单 3 条例外已登记）
DECL_EXIT=0
```

新增白名单 1 条：`reference` —— 写入端调 `GraphEdge::relation(…)`，
字面量 `"reference"` 内置在 `graph_dtos.rs:85` 的构造器体内
（**好处正是「reference」只有一处字面量**）。

### 6.5 备份自证（`cp` 成功 ≠ 备份是「改造前」）

纪律要求备份**自证是改造前快照**（取备份时机若落在「已改一部分」之后，回滚会得到**半改状态**）。
判据是双向的：只可能出现在**新内容**里的符号必须 **0 命中**；只可能出现在**旧内容**里的符号必须 **> 0**。

固化脚本 `output/verify-backup-pre-2026-09-14.mjs`（可重跑）实测：

```
✔ src-tauri_crates_harness_src_knowledge_graph.rs.pre
✔ src-tauri_crates_harness_src_graph_dtos.rs.pre
✔ src-tauri_crates_dao_src_repo_knowledge_graph.rs.pre
✔ src-tauri_src_commands_wiki.rs.pre
✔ src_components_wiki_GraphView.tsx.pre
✔ src_pages_WikiGraphPage.tsx.pre
✔ page_type.rs.pre
✔ check-decl-evidence.mjs.pre
✅ 8 份备份全部自证为「改造前」（新符号 0 泄漏 + 旧符号全在）
EXIT=0
```

正样本对照（备份里能看到旧形态本体）：

```
wiki.rs.pre:1128-1132
  mapping_edges.push(GraphEdge {
      source: entity_node.id.clone(),
      target: note_id.clone(),
      edge_type: "mapping".to_string(),     ← 改造前：与 source/target 挤在一个结构体字面量里
  });
```

> ⚠ **第一版断言 5/8 假红，全部是我自己写错**（这正是「反向断言须锚定被测对象自身形态」）：
> ① **符号名靠猜** —— 我写了 `UnrecognizedTypeStat` / `unresolvedTypeStats`，
> 真名是 `UnresolvedTypeStat` / `unresolvedStats`；
> ② **子串重叠** —— `refresh_unresolved` 会命中 `refresh_unresolved_types`，
> 于是一边报「泄漏新符号」一边又满足「旧符号存在」，双向同时误判。
> **教训：负对照自己也要有负对照。**

---

## 7. 门禁与测试复跑（**全部为本轮实测**，无一条沿用既往记录）

| 门禁 | 命令 | 结果 |
|---|---|---|
| 接线守卫 | `node scripts/check-port-axiom-wiring.mjs` | ✅ `EXIT=0`｜已接 **15** / 未接 **0** |
| 引用核验（严格） | `node scripts/check-decl-evidence.mjs` | ✅ `EXIT=0`｜**35** 条可定位 / **0 腐烂** / 0 存疑 / 白名单 3 |
| 引用核验（自测） | `node scripts/check-decl-evidence.mjs --selftest` | ✅ `EXIT=0`｜8 例分类 + **3 例覆盖率负样本**全过 |
| 全量编译 | `cargo check --workspace --all-targets` | ✅ `CHECK_EXIT=0`（`Finished` 13m39s，**这是 `--all-targets`，含测试目标**） |
| 契约层测试 | `cargo test -p axagent-harness --lib` | ✅ `HT_EXIT=0`｜**681 passed / 0 failed** |
| 写端观测测试 | `cargo test -p axagent-dao --lib entity_type_observation` | ✅ `DO_EXIT=0`｜**3 passed / 0 failed** |
| 边侧端到端探针 | `cargo run -p axagent-dao --example wiki_edge_relation_probe -- <pg-url>` | ✅ `PROBE_EXIT=0`｜四步符合预期，数字与 §3.2 逐字一致 |
| 本体一致性 | `node scripts/check-ontology-consistency.mjs` | ✅ `ONTO_EXIT=0`｜副本与权威源一致；**2 项口径分歧待裁决**（既有登记） |
| 成对字段 | `node scripts/check-paired-fields.ts` | ✅ `PAIRED_EXIT=0` |
| 跨层契约 | `node scripts/check-contracts.mjs` | ✅ `CONTRACTS_EXIT=0`｜`errors=0 warnings=6` |
| 国际化 | `python scripts/check_i18n.py --strict` | ✅ `I18N_EXIT=0`｜代码引用 key **9691** 全部有定义 |
| i18n key 落地 | 逐 key 读 11 个 locale（本轮实测） | ✅ 5 key × 11 = **55 处全在** |
| 前端类型 | `npx tsc --noEmit` | ✅ `TSC_EXIT=0` |
| 备份有效性 | `node output/verify-backup-pre-2026-09-14.mjs` | ✅ `EXIT=0`｜**8/8 自证为「改造前」** |
| 记忆三层 | `node .workbuddy/memory/mem.mjs check` | ✅ `EXIT=0`｜注入层 7103 B / 7168 B（余 65 B）；判据层 **158 条** |

> ⚠ 记录一条**方法学**提醒：上表的 `cargo check --workspace --all-targets` 之所以必须显式带
> `--all-targets`，是因为**日志不分 target** —— 不加它时 `Finished` 并不证明测试目标被编译过
> （判据 #72「编译通过」三型假阳性之一）。本轮的编译错误恰恰只出现在**测试构造点**
> （`GraphEdge` 新增字段后 `E0063 missing field relation_type`），所以这一条不是形式要求。

---

## 8. 未完成 / 阻塞 / 只登记不改

### 8.1 只登记不改（本轮的「报告不擅改」）

| 文件:`行` | 内容 | 处理 |
|---|---|---|
| `docs/audits/AUDIT-ontology-abc-implementation-2026-09-14.md:131` | `dao/src/repo/knowledge_graph.rs:716` 排除因果边 | **不擅改**。真值已移至 **:811**。该报告是**当时快照**，`605`/`716` 在写下时都正确 |
| 同上 `:150` | `dao/src/repo/knowledge_graph.rs:605` `upsert_relation` 校验 | 同上。真值已移至 **:704**（备份 `output/backup-2026-09-14g/…knowledge_graph.rs.pre:605` 已自证「改造前」确实是该行） |
| 同上 `:118/344/542` | `commands/knowledge.rs:1725` | 同上。真值 **1738** |
| 同上 `:342` | `harness/src/knowledge_graph.rs:317` | 同上 |

> **对照表（旧 → 新）**：`knowledge_graph.rs` 716→811、605→704、1226→1349；
> `commands/knowledge.rs` 1723→1738、1725→1738、1752→1767、1765→1782、1767→1782、1781/1783→1798；
> `commands/wiki.rs` 1131→1132、1383→1396、1637→1650、1883→1896；
> `GraphView.tsx` 115→116。读旧报告时按此表换算。

### 8.2 既有失败，与本次改动**无关**（只登记）

| 项 | 现象 | 判定依据 |
|---|---|---|
| `cargo test -p axagent-dao --lib` 3 个迁移测试 | `assertion left: 228, right: 227` | `CURRENT_VERSION: i32 = 227` 与其描述项注释「写 228」**自相矛盾**，早于本会话；本轮新测试单跑 `3 passed` |
| `check-layer-discipline.mjs` | 超基线 3 处 | 全部来自**并发会话文件**（`stock_workflow/sim_hook.rs` 3、`rhai_pm.rs` 1）⇒ 按纪律**禁止改动** |
| `check:serde` | `EXIT=1` | 报的是 `analysis-engine/src/analyst_feedback/*` 既有结构体，非本次改动文件 |

### 8.3 待产品决策（不擅自动手）

- **P3**：是否给 `GraphEnhancedSearchInput` 的 filters 接上入边（会改变 RAG 召回面）。
- **P3 附带**：`rag.rs:676` 是否对 `entity_type` 做归一后再入 prompt（会改变 prompt 内容）。
- **P1**：未登记实体类型是「让日志可见」还是「阻断导入」（当前：只可见，不阻断）。

---

## 9. 本轮改动文件清单

**后端**

- `src-tauri/crates/harness/src/knowledge_graph.rs`（`EDGE_TYPE_DECLS` 6 条 + `is_declared_edge_type` + `uninterpreted_edge_label` + 4 个测试；引用行号修正）
- `src-tauri/crates/harness/src/graph_dtos.rs`（`GraphEdge.relation_type` + 2 构造器；`GraphData.unresolved_relations`；`refresh_unresolved`；`group_by_raw_label` 收敛；4 个测试）
- `src-tauri/crates/harness/src/page_type.rs`（6 处引用行号修正）
- `src-tauri/crates/dao/src/repo/knowledge_graph.rs`（`normalize_relation_label` + 出口改 `GraphEdge::relation`；`observe_violation` / `unregistered_entity_types` / `observe_entity_types`；4 个写入点接入；3 个测试）
- `src-tauri/crates/dao/src/repo/note.rs`（笔记链接边改 `GraphEdge::structural`）
- `src-tauri/crates/dao/examples/wiki_edge_relation_probe.rs`（**新建**，四步端到端探针）
- `src-tauri/src/commands/knowledge.rs`（CSV 导入直写路径接观测）
- `src-tauri/src/commands/wiki.rs`（mapping 合成边改构造器；`refresh_unresolved`）
- `src-tauri/crates/agent/src/relevance.rs`、`src-tauri/crates/dao/src/repo/louvain.rs`（测试字面量改构造器）

**前端**

- `src/components/wiki/GraphView.tsx`（`relationType?` + `relationLegend` + 图例分节）
- `src/pages/WikiGraphPage.tsx`（边侧 Alert）
- `src/i18n/locales/*.json` ×11（5 key each）

**脚本 / 产物**

- `scripts/check-decl-evidence.mjs`（`DECL_TABLES` 单份来源 + `coverageGaps` 双向自证 + 3 负样本 + 白名单 +1）
- `output/fix-citation-rot-2026-09-14.mjs`（**新建**，幂等 + 自证）
- `output/verify-backup-pre-2026-09-14.mjs`（**新建**，8 份备份自证「改造前」）
- `output/backup-2026-09-14g/`（8 个文件 `.pre` 备份 + 旧日志留档 + 污染现场取证 `2026-09-14.md.mangled`）

**外部资产**

- `~/.workbuddy/skills/rust-consistency-audit/`（**方法总纲**）：
  `references/method-26-ontology-placement-audit.md` 新增**技法 11**（「接线做完之后真缺陷才开始浮出来」五条）
  与「引用腐烂三型新坑」小节；`SKILL.md` 护栏 **三条 → 四条**（新增「清单类门禁必须自证覆盖率」）、
  §① 分流表补技法 11 触发词（`description` 保持 1061 字符，未按整段判据回填）。
