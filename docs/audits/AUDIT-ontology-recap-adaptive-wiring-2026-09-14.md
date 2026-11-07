# 复盘：本体落地后的**业务层 / 基础能力层适应性接线**审计（2026-09-14）

> 状态：active ｜ 日期：2026-09-14 ｜ 对应版本：v2.9.7


> 触发问题：**「复盘会话中任务是否存在不足，特别是在 ontology 模式下，业务层及其它基础能力层是否要进行适应性接线」**
>
> 本轮**不改业务语义、不做产品决策**，只回答两件事：
> ① 本体的每个公开 API **真实的调用方是谁**（有没有人用）；
> ② 哪些地方**该接线而没接**，以及接在哪一层才对。
>
> 纪律：**不沿用既往记录的结论** —— 矩阵里每一格都用当轮实测重新取数（含真 DB）。

---

## 0. 方法与取证

| 手段 | 说明 |
|---|---|
| 调用方计数 | 对本体每个公开 API 做全仓 grep，**人工剔除定义处 / 注释 / `target/` 构建产物 / 测试** |
| 入口枚举 | 对校验类 API，反查「所有写入路径」而非只看被调用的那一条 |
| 真 DB 取值分布 | `output/onto-gap-probe.mjs`（只读；连 `~/.axagent/db_config.json` 指向的 `localhost/axinvest`） |

DB 侧当轮实测（原样）：

```
notes.page_type                        : '' 48575 ｜ doc 15
knowledge_entities.entity_type         : person 62616 ｜ company 9564 ｜ concept 487 ｜ industry 146 ｜ COMPANY 2 ｜ CONCEPT 1
knowledge_relations                    : 112937 行 / 56 distinct
  ├ 非 ASCII                           : 53 distinct / 74325 行
  └ ASCII                              : 3 distinct（has_concept 28326 ｜ in_industry 9498 ｜ mentions 788）
relation_type like '"%'（D5 残留）      : 0 行
page_type='doc' 的笔记                 : 15 行，**15 行全有 vault_id**
```

---

## 1. 结论总览：本体资产 × 消费端矩阵

| # | 本体资产 | 定义处 | 生产消费方 | 判定 |
|---|---|---|---|---|
| 1 | `PageType` 词汇表（14 变体） | `page_type.rs:56` | `wiki_compiler.rs:678`、`graph_dtos.rs:167`/`:315` | ✅ 已接 |
| 2 | `SAME/CROSS_TYPE_AFFINITY` 公理 | `page_type.rs:169`–`:200` | `graph_dtos.rs:495`–`:501` → `graph_insights.rs:136`/`:237` | ✅ 已接（作用面窄：只进相关性评分） |
| 3 | `RELATION_DECLS` 的 **id** 部分 | `knowledge_graph.rs:337` | `validate_relation_id` ← `knowledge.rs:1805`、`dao/knowledge_graph.rs:605` | ✅ 已接 |
| 4 | `RELATION_DECLS` 的 **domain/range** 部分 | 同上 | **无** —— `validate_relation_write` 仅 4 处测试 | ❌ **零消费** |
| 5 | `ENTITY_TYPE_DECLS`（11 条） | `knowledge_graph.rs:600` | **无** —— `validate_entity_type` 仅 4 处测试 | ❌ **零消费** |
| 6 | `DATA_DRIVEN_COLUMN` 形态判据 | `knowledge_graph.rs:563` | 经 `is_declared_relation` 第三支 ← `validate_relation_id` | ✅ 已接（间接） |
| 7 | `HANDLE_DECLS` + `validate_port_axioms` | `workflow_port_axioms.rs` | `workflow_template.rs:973` —— **仅手动保存** | ⚠️ **接错层**（见缺口 1） |
| 8 | `domain_ontology::BOTTLENECK_WEIGHTS` | `domain_ontology.rs` | `seed_serenity.rs:1268` | ✅ 已接 |
| 9 | `domain_semantics::{Percent100, Ratio01}` | `domain_semantics.rs` | `evidence_citation.rs:24` | ⚠️ newtype 仅接 1/24（既有登记） |
| 10 | `GraphData.unresolved_types`（A2） | `graph_dtos.rs:56` | `wiki.rs:1149` → 前端 banner | ✅ 已接，但**只覆盖节点侧**（见缺口 4） |
| 11 | 三张表自检（`relation_table_ok` 等 5 个） | `knowledge_graph.rs:796`+ | **仅测试**（作为覆盖制门禁可接受，前提是 harness 单测在 CI 内） | ◻ 设计如此 |

**一句话结论**：本体的**读端**接得不错（1/2/3/6/8/10 都有生产消费）；**写入端与可视化端**是主要空白 —— 这正是「本体建在契约层、但没有接进业务的写路径」的形态。

---

## 2. 缺口清单（按严重性）

### 缺口 1（P0）｜端口公理门禁只覆盖 1/6 入口，且**最容易被绕过的那条恰好没覆盖**

`validate_port_axioms` 的唯一生产调用点是 `workflow_template.rs:973`，它属于 **`validate_workflow_template`（`pub async fn`，line 595）** —— 而这是一个**Tauri 命令**（注册于 `register_commands.rs:1527`），全仓**唯一消费方是前端** `workflowEditorStore.ts:997`。

于是「谁在写工作流」这个集合与「谁被校验」这个集合几乎不相交：

| # | 写入入口 | 实现 | 过公理校验？ |
|---|---|---|---|
| 1 | UI **手动**保存 | `WorkflowEditor.tsx:899` → `validateTemplate()` → `!isValid` 则 `return` | ✅ **过**（真挡库） |
| 2 | UI **自动**保存 | `useWorkflowAutoSave.ts:38` `doSave()` | ❌ **不过** |
| 3 | 后端命令 `create_workflow_template` | `workflow_template.rs:230`（内部零校验，直接 `insert`） | ❌ 不过 |
| 4 | 后端命令 `update_workflow_template` | `workflow_template.rs:305` | ❌ 不过 |
| 5 | **种子写入** `seed_production.rs` | `OPC_TEMPLATE_VERSION` 触发重种子 | ❌ 不过 |
| 6 | 导入（`do_import_workflow` / `import_n8n_directory`）、`save_dynamic_workflow` | `:2581`/`:2841`/`:2934`/`:3019` | ❌ 不过 |

**两条实证链**：

- **入口 2 是实际主路径**：`useWorkflowAutoSave` 的 `isDirty` 一变就起 5 秒定时器（`:9` `AUTO_SAVE_DELAY_MS = 5000`，`:135`–`:148`），到点直接 `invoke("update_workflow_template")` —— 用户在画布上拖一条边、handle 选错，**5 秒后非法边就已落库**；而 `isDirty` 被清 ⇒ 他之后甚至不会再触发一次手动保存，**校验可能永远不报**。
- **入口 5 已经被打穿过一次**：本轮修的 `s-gonogo` 真缺陷（`default_case: Some("NO-GO 打回")` 用 label、出边 `source_handle` 用 value）就是**从种子进来的**，而且是我靠**审计脚本**（`port_axiom_audit.rs`）发现的，**不是被校验拦下的** —— 这本身就是「门禁不在写路径上」的直接证据。

**判定**：门禁目前是**客户端自检**，不是**服务端强制**。本体（`validate_port_axioms`）放在 harness 层是对的；错的是**接线点选在了前端 action**。

**建议接线（推荐做法）**：
把 `validate_port_axioms` 的 `Error` 级判定下沉到**后端写入函数** `create_workflow_template` / `update_workflow_template`（以及种子/导入各自的落库前一步），前端保留 `validate_workflow_template` 仅作**即时反馈**（用户不必等到保存才知道）。
- 关键约束：**分级判据不得写两份** —— 直接复用 `v.severity()`，不要在后端再写一遍 `matches!`。
- 行为变更需显式提醒：下沉后，**存量非法模板（含 `prod-startup-mvp` 这类 v3 数据）在下次被保存时会失败**；建议 error 只在「新增/改动的那条边」上生效，或先做一次存量清单（`port_axiom_audit` 已能产出，实测 111 模板 / 3 处 error / 全在 `prod-startup-mvp`）。

### 缺口 2（P1）｜`ENTITY_TYPE_DECLS` 零消费，而消费端正在用「精确小写」比字面量

- 本体侧：`validate_entity_type`（`knowledge_graph.rs:736`）**生产 0 调用**，只有 4 处测试。
- 消费端侧：`analysis-engine/src/knowledge_loader.rs` 用**硬编码字面量**判断：
  - `:73` `if e.entity_type == "concept" || e.entity_type == "industry"`
  - `:87` `.map(|t| t == "company")`
  - `:90` `.map(|t| t == "concept" || t == "industry")`
- DB 侧：`entity_type` 实测有 **`COMPANY` 2 行 / `CONCEPT` 1 行**（大小写变体）。

⇒ 这三行合起来是一条**静默漏配**：大写变体在 `==` 比较下**恒 false** ⇒ 对应实体不会被注册为概念节点 / 不会被识别为 company ⇒ 概念成员关系**少建**。而因为大小写变体已被本体**登记为合法观察值**（`observed: true`），`validate_entity_type` 即便被接上**也不会报它** —— 「登记」这个动作**掩盖**了「消费端漏配」。

**判定**：应接，且接法有两步（缺一不可）：
1. **写端**：在实体写入路径（LLM 抽取 `ingest_pipeline.rs:79`、CSV 导入 `knowledge.rs`）调 `validate_entity_type` + `warn_on_violations` —— 让新的未登记/异形值**在进入时**就可见；
2. **读端**：`knowledge_loader.rs` 的三处精确比较改为**经本体归一**（如 `entity_type_decl(t)` 命中后按登记的规范 id 比较，或至少 `eq_ignore_ascii_case`）—— 否则「只登记不归一」等于没管。

> 影响面标注：3 行，**未验证**这 3 行是否落在某个 kb 的加载范围内 ⇒ 严重性 = 机制成立、规模待测。

### 缺口 3（P1）｜`RELATION_DECLS` 的 domain/range 零消费

18 条登记里有实质结构信息的是 `domain` / `range`（`ENTITY_ONLY` 等），`validate_relation_write`（`:748`）是唯一消费入口，**生产 0 调用**（4 处测试）。

现状：写入端只调 `validate_relation_id`（只校验**名字**，不校验**两端节点类**）。因此「公司 —owns→ 概念」这类**违反值域**的边**没有任何检查**。

**接不上是有客观原因的**（值得记下来，不是纯遗漏）：`trajectory/src/storage.rs::save_relationship` **只传实体 id、不传节点类**（这正是那 11 条记忆变体登记 `domain: None` 的原因）。所以这项要接，前提是**先让写入方拿到节点类** —— 属于**签名变更**，不是补一行。

**判定**：登记为「需先改写入方签名」的接线项，本轮不建议直接动。

### 缺口 4（P2）｜前端词汇表未跟随 D3 扩展，**且 D3 让 6 类节点退出了 A2 的视野**

这是一条**对称双缺口**，节点侧与边侧都存在，而 A2 只做了「节点类型 ⊄ 后端词表」这一格：

| | 后端词表内 | 后端词表外 |
|---|---|---|
| **前端有样式** | ✅ 正常 | ⚠️ A2 的 banner 会报 |
| **前端无样式** | ❌ **无任何信号**（本轮盲区） | ❌ A2 报 + 前端 fallback |

- **节点侧**：后端 `PageType` 14 变体；前端 `GraphNodeType` 只有 **4** 个（`note`/`concept`/`entity`/`source`），其余落 `typeMap[node.type] \|\| typeMap.note`（`graphViewUtils.ts:191`）⇒ **静默变成 note 配色**。
  实测 `page_type='doc'` 有 **15 行且全有 vault_id** ⇒ **确实会进图谱**，即这 15 个节点现在以 note 色显示。
  ⚠ **本轮（D3）的副作用**：D3 把 `doc`/`daily`/`knowledge`/`knowledge_document`/`synced`/`log` 纳入词汇表后，`unresolved_types` **不再报它们** ⇒ 这 6 类节点从「A2 会提示」变成「谁都不提示」，而前端配色依旧缺失。**这是收敛动作把一批节点从可见推入不可见**，必须显式告知。
  （上轮 `GraphView.tsx:102`–`:113` 的注释已经点破「两者是不同集合」，但当时判定为产品决策而**只登记未实施** —— 本轮把它升格为「应收口的缺口」。）
- **边侧**：后端 `GraphEdge.type` 在 Wiki 图谱里有**两种来源** —— 笔记链接恒为 `"link"`（`note.rs:563`），而**融合进来的知识库实体边是 DB 的 `relation_type` 原样透传**（`wiki.rs:1104`–`:1108`，只加 `entity:` 前缀）。
  前端 `GraphEdgeType` 只列 **6** 个，其余落 `edgeStyles[e.type] \|\| edgeStyles.link`（`GraphView.tsx:786` 与 `:1105`）。
  ⇒ DB 里 **53 个非 ASCII / 74325 行边**（占该表 65.8%）**全部渲染成 link 样式**，与真正的笔记链接**视觉上不可区分**；而**边侧没有任何 A2 式的可观测化**。

**判定**：应接，建议**先做「可见性」再做「配色」**（两次独立改动，不等产品选色）：
1. 把 A2 的可观测化做成**节点 + 边对称**：边侧也产出一份「`relation_type` 不在登记表 / 属数据驱动形态」的统计并进同一 banner（后端已有 `is_declared_relation`，可复用；**判据不得写第二份**）。
2. 前端配色/i18n 跟随词表属**产品决策**（要选色、要 11 语言翻译），建议与「前端无样式的类型」一并收口，而不是逐个补。

### 缺口 5（P2）｜OPC 业务域自带一套 `entity_types`，与本体**同名不同义**

`analysis-engine/src/opc/industry_config.rs` 为 **12 个行业**各定义了一套 `entity_types`：

```
accounting        : invoice / finance_record / customer
finance_invest    : stock / portfolio / order
software_dev      : requirement / task / bug / release
design            : design / mockup / asset
ai_research       : paper / experiment / report
content_media     : article / video / post
ecommerce         : product / order / customer
education         : course / student / assignment
gis               : dataset / layer / analysis
consulting        : project / client / report
project_management: project / task / milestone
sales_growth      : lead / deal / campaign
security          : incident / audit / policy
game_dev          : feature / asset / build
```

与 `ENTITY_TYPE_DECLS` 登记的 KG 实体类型（`person`/`company`/`industry`/`concept`…）**值域完全不相交**，但**字段同名**（都叫 `entity_type`）。

**准确判定（不要说成「OPC 无校验」）**：OPC 有**自己的**校验体系 `industry_validator.rs`（按行业分派 `validate_entity`），所以这是**平行体系 / 架构分裂**，不是零防护。性质属判据 K 的「**一名多义**」：同一名字在两个层级指两套概念。

**建议**：不必强行合并（一个是「知识图谱实体」，一个是「业务记录类型」，语义层级不同）。但应二选一收口：
- **推荐**：把 OPC 的 `entity_types` 也**纳入本体登记**（作为**另一个命名空间**，如 `OpcRecordTypeDecl`），使「项目里到底有几套实体类型词表」这一事实**可枚举**；
- 或者至少**改名**（如 `record_types`）消除同名 —— 成本更低，但治标。

### 缺口 6（P3）｜`crates/search` 对本体零引用

`crates/search/src` 对 `PageType` / `knowledge_graph::*` **0 命中** ⇒ 检索层不区分页面载体类型与关系类型。

**判定**：**不一定是缺陷** —— 若检索按全文/向量召回，本体的类型信息本就不参与排序。**登记为待判断项**（需产品确认「检索结果是否应按 page_type 分层」），本轮不动。

---

## 3. 顺带更正：一处**登记口径错配**（已修）

`DATA_DRIVEN_COLUMN` 的 `column` 字段声明的是 **DB 列** `knowledge_relations.relation_type`，但其文档与 `meaning` 用的数字是 **数据文件口径**（`edges.csv` 的 `rtype` = 24 distinct / 其中 22 个中文 / 「约 6 万行」）。

**错配后果**：校验器面对的是 DB 全表，而注释只给了文件口径 ⇒ 会让人误以为「非 ASCII 值可枚举（22 个）」，从而考虑改成白名单 —— 那会**静默漏掉另外 31 个值**（DB 实测非 ASCII **53** 个 / **74325** 行，比 CSV 多出的来自记忆实体图与 LLM 抽取路径）。

**修法**：在 `DATA_DRIVEN_COLUMN` 的 doc 里并列**两个口径**并标注「判据依据必须取 DB 全表」，`meaning` 字段同步；改动为纯注释/字符串，无行为变更。备份 `output/backup-2026-09-14e/knowledge_graph.rs.pre-recap`（自证：`74325` 命中 0）。

---

## 4. 建议接线清单（按性价比排序）

| 优先级 | 动作 | 落点 | 前提 / 风险 |
|---|---|---|---|
| **P0** | 端口公理 `Error` 级判定下沉到后端写入函数 | `workflow_template.rs:230` / `:305`（+ 种子、导入各自落库前） | 存量非法模板会在下次保存时失败 ⇒ 需先出存量清单；分级判据复用 `v.severity()` |
| **P1** | 实体类型「写端校验 + 读端归一」成对做 | 写：`ingest_pipeline.rs` / `knowledge.rs` 导入；读：`knowledge_loader.rs:73/87/90` | 只做写端不算闭环（登记会掩盖漏配） |
| **P1** | `domain/range` 校验 | `validate_relation_write` | **需先改写入方签名**（现只传实体 id，无节点类）⇒ 不建议本轮动 |
| **P2** | 边侧可观测化（与 A2 对称） | 后端产统计 → 复用现有 banner | 判据复用 `is_declared_relation`，不得写第二份 |
| **P2** | 前端配色 / i18n 跟随词表 | `GraphView.tsx` / `graphViewUtils.ts` + 11 locale | **产品决策**（选色 + 翻译）⇒ 整批收口，别逐个补 |
| **P2** | OPC `entity_types` 纳入登记或改名 | `opc/industry_config.rs` | 建议作为独立命名空间登记；改名成本更低但治标 |
| **P3** | `search` 层是否应消费本体 | `crates/search` | 需产品确认检索是否按类型分层 |

> **⚡ 执行状态（2026-09-14 追加，仅登记不修改上文结论）**
> 本清单已按用户指令「按照 p0、p1、p2、p3 顺序执行」实施完毕，**执行记录见**
> [`AUDIT-ontology-p0123-execution-2026-09-14.md`](AUDIT-ontology-p0123-execution-2026-09-14.md)。
> **三处与原计划的偏差，读本表时须知道**：
> 1. **P2「边侧可观测化」的真实缺陷与预期不同** —— 不是「前端配色没跟上」，
>    而是后端出口把 `relation_type` **读出来又扔掉**（`edge_type` 恒为常量 `"reference"`），
>    实测 **112937 行 / 56 个关系类型全塌成 1 个值**。属**纯 bug**（信息丢失，非产品决策），已顺手修。
> 2. **P2「前端配色」实际未做**（仍是产品决策，未替用户选色）。改为给 `relation_type` 一个
>    **唯一展示出口**（图例 top-8 分布，**刻意不做白名单**）——配色的决定权仍留给产品。
> 3. **P1 的 `domain/range` 校验按本表建议未动**（前提未满足：写入方仍只传实体 id）。
>
> 另：本轮在改代码时**自己造成了 19 条 `文件:行` 引用腐烂**并已全部修复（含 `page_type.rs`
> 三处**此前从未被人核过**的引用），同时在检查器里补了「清单覆盖率自证」——
> 详见执行记录 §6。

---

## 5. 验证边界（如实登记）

1. **本轮未改任何业务语义**：唯一代码改动是 §3 的注释/字符串口径补正；**未实施**上表任何接线（用户本轮问的是「是否存在不足 / 要不要接」，不是「去接」）。
2. **缺口 2 的规模未测**：`COMPANY`/`CONCEPT` 共 3 行，**未验证**其是否落在被加载的 kb 范围内 ⇒ 机制成立、影响面待测。
3. **缺口 1 的「自动保存不过校验」是读码结论 + 入口枚举**得出的，未做运行期实验（如「自动保存后 DB 里出现非法边」的端到端复现）⇒ 建议下一步用一条**故意断链的边**做端到端证实。
4. **DB 取数为只读**，脚本 `output/onto-gap-probe.mjs`；未写、未迁移任何数据。
5. 门禁现状：`mem.mjs check` 绿；`scripts/check-decl-evidence.mjs`（若跑）应仍绿 —— §3 改动未触碰 `evidence` 字段。
